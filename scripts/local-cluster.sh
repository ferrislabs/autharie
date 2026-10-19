#!/usr/bin/env bash
# Creates a k3d cluster dedicated to Autharie and installs what the operator
# needs to reconcile an IdentityInstance.
#
# It never touches the current kubectl context: every command targets the
# cluster's own context explicitly. A laptop usually has other clusters on it,
# and installing an operator into the wrong one is not something you notice
# until later.
set -euo pipefail

CLUSTER_NAME="${AUTHARIE_CLUSTER_NAME:-autharie-local}"
CONTEXT="k3d-${CLUSTER_NAME}"
CNPG_VERSION="${CNPG_VERSION:-1.25.1}"
# Envoy Gateway's chart brings the Gateway API CRDs with it, so this one
# version pins both. Installing the upstream CRDs separately as well is how you
# get two sources for the same CRD and a version nobody can state.
ENVOY_GATEWAY_VERSION="${ENVOY_GATEWAY_VERSION:-1.2.6}"
# Holds Herald's replica floor. Installed here and not by the data plane chart:
# a ScaledObject cannot be created by the release that installs its own CRD.
KEDA_VERSION="${KEDA_VERSION:-2.21.0}"
# Not 8080: a developer machine usually already has something on it, and k3d
# fails the whole cluster creation on a port collision rather than picking
# another one.
HTTP_PORT="${AUTHARIE_HTTP_PORT:-8081}"
HTTPS_PORT="${AUTHARIE_HTTPS_PORT:-8444}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "❌ $1 is required but not installed" >&2
        exit 1
    }
}

usage() {
    cat <<USAGE
usage: $(basename "$0") <up|down|status|host-entry>

  up          create the cluster and install the CRDs, CloudNativePG, Envoy Gateway and KEDA
  down        delete the cluster
  status      show what is installed
  host-entry  make sure pods resolve host.k3d.internal (lost when Docker restarts)

Environment:
  AUTHARIE_CLUSTER_NAME   cluster name (default: autharie-local)
  AUTHARIE_HTTP_PORT      host port mapped to the ingress (default: ${HTTP_PORT})
  AUTHARIE_HTTPS_PORT     host port mapped to TLS (default: ${HTTPS_PORT})
  CNPG_VERSION          CloudNativePG version (default: ${CNPG_VERSION})
  ENVOY_GATEWAY_VERSION Envoy Gateway version (default: ${ENVOY_GATEWAY_VERSION})
  KEDA_VERSION          KEDA chart version (default: ${KEDA_VERSION})
USAGE
}

up() {
    require k3d
    require kubectl
    require helm

    for port in "${HTTP_PORT}" "${HTTPS_PORT}"; do
        if lsof -nP -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1; then
            echo "❌ port ${port} is already in use." >&2
            echo "   Override it: AUTHARIE_HTTP_PORT=... AUTHARIE_HTTPS_PORT=... make local-up" >&2
            exit 1
        fi
    done

    if k3d cluster list "${CLUSTER_NAME}" >/dev/null 2>&1; then
        echo "✅ cluster ${CLUSTER_NAME} already exists"
    else
        echo "📦 creating k3d cluster ${CLUSTER_NAME}"
        # Traefik is disabled: the operator serves every instance through an
        # HTTPRoute on the Envoy Gateway installed below, and two
        # LoadBalancers cannot both hold port 80 on the node. The loser stays
        # Pending and says nothing about why.
        # --kubeconfig-switch-context=false: k3d switches the active context on
        # create by default, which would silently repoint every kubectl in the
        # shell at a cluster the user did not ask to be in.
        k3d cluster create "${CLUSTER_NAME}" \
            --agents 1 \
            --port "${HTTP_PORT}:80@loadbalancer" \
            --port "${HTTPS_PORT}:443@loadbalancer" \
            --k3s-arg "--disable=traefik@server:*" \
            --kubeconfig-switch-context=false \
            --wait
    fi

    echo "🔌 installing CloudNativePG ${CNPG_VERSION}"
    # The operator provisions each deployment's database as a
    # postgresql.cnpg.io/v1 Cluster, so without this every IdentityInstance
    # stalls in DatabaseProvisioning with no obvious cause.
    kubectl --context "${CONTEXT}" apply --server-side -f \
        "https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-${CNPG_VERSION%.*}/releases/cnpg-${CNPG_VERSION}.yaml"

    echo "⏳ waiting for CloudNativePG to be ready"
    kubectl --context "${CONTEXT}" -n cnpg-system wait --for=condition=Available \
        deployment/cnpg-controller-manager --timeout=180s

    if kubectl --context "${CONTEXT}" -n kube-system get svc traefik >/dev/null 2>&1; then
        echo
        echo "⚠️  this cluster still runs Traefik, from before instances moved to"
        echo "   Gateway API. It holds port 80 on the node, so the Gateway's"
        echo "   service will sit in Pending until it is gone:"
        echo
        echo "     kubectl --context ${CONTEXT} -n kube-system delete helmchart traefik"
        echo "     kubectl --context ${CONTEXT} -n kube-system delete svc traefik"
        echo
        echo "   Nothing is served through an Ingress any more, so removing it"
        echo "   costs nothing. Recreating the cluster does the same thing."
        echo
    fi

    echo "🚪 installing Envoy Gateway ${ENVOY_GATEWAY_VERSION}"
    # The data plane chart declares a GatewayClass and a Gateway; without this
    # controller they are accepted and never programmed, which reads as a
    # working edge right up until a route attaches to it and serves nothing.
    helm --kube-context "${CONTEXT}" upgrade --install envoy-gateway \
        oci://docker.io/envoyproxy/gateway-helm \
        --version "v${ENVOY_GATEWAY_VERSION}" \
        --namespace envoy-gateway-system --create-namespace \
        --wait --timeout 5m

    echo "⏳ waiting for Envoy Gateway to be ready"
    kubectl --context "${CONTEXT}" -n envoy-gateway-system wait --for=condition=Available \
        deployment/envoy-gateway --timeout=180s

    echo "📈 installing KEDA ${KEDA_VERSION}"
    helm --kube-context "${CONTEXT}" upgrade --install keda keda \
        --repo https://kedacore.github.io/charts \
        --version "${KEDA_VERSION}" \
        --namespace keda --create-namespace \
        --wait --timeout 5m

    # The examples deploy into this namespace; creating it here keeps the
    # first run from failing on something unrelated to Autharie.
    echo "📁 creating the test-autharie namespace"
    kubectl --context "${CONTEXT}" create namespace test-autharie \
        --dry-run=client -o yaml | kubectl --context "${CONTEXT}" apply -f - >/dev/null

    echo "📄 installing Autharie CRDs"
    "${REPO_ROOT}/scripts/generate-crds.sh" >/dev/null
    kubectl --context "${CONTEXT}" apply -f "${REPO_ROOT}/k8s/crds/"

    host_entry

    echo
    echo "✅ ${CLUSTER_NAME} is ready"
    echo
    echo "   The context is NOT switched. Point commands at it explicitly:"
    echo "     kubectl --context ${CONTEXT} get identityinstances -A"
    echo
    echo "   Run the operator against it:"
    echo "     KUBECONFIG=\$(k3d kubeconfig write ${CLUSTER_NAME}) cargo run -p autharie-operator"
    echo
    echo "   Then apply an example:"
    echo "     kubectl --context ${CONTEXT} apply -f k8s/examples/identity-instance-ferriskey.yaml"
}

# The host as seen from the cluster's Docker network.
#
# Resolved rather than assumed, because every runtime answers differently:
# OrbStack serves host.docker.internal as 0.250.250.254, Docker Desktop as an
# address of its own, and plain Docker on Linux not at all -- there the
# network's gateway is the host. IPv4 only: NodeHosts wants one address, and
# CoreDNS answering an A query with nothing is the failure this is here to fix.
host_gateway_ip() {
    local network="k3d-${CLUSTER_NAME}" ip

    ip="$(docker run --rm --network "${network}" alpine \
        getent hosts host.docker.internal 2>/dev/null \
        | awk '$1 ~ /^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$/ { print $1; exit }')" || true

    if [ -z "${ip}" ]; then
        ip="$(docker network inspect "${network}" \
            --format '{{range .IPAM.Config}}{{.Gateway}} {{end}}' 2>/dev/null \
            | tr ' ' '\n' \
            | awk '$1 ~ /^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$/ { print $1; exit }')" || true
    fi

    printf '%s' "${ip}"
}

# k3d writes `host.k3d.internal` into CoreDNS's NodeHosts when it creates or
# starts a cluster. When Docker or OrbStack restarts, the node containers come
# back without `k3d cluster start`, k3s rewrites NodeHosts from its nodes, and
# the entry is gone. Every pod that reaches this machine by that name -- Herald
# first -- then fails on DNS and says little about it: on 2026-10-17 a Herald
# had been silent for eight days.
#
# Only added when missing. An entry that is there was put there by k3d, which
# knows its runtime better than a guess here does; whether it actually works is
# for the caller to check, with a request.
host_entry() {
    require kubectl
    require docker
    require jq

    local hosts ip patch

    hosts="$(kubectl --context "${CONTEXT}" -n kube-system get configmap coredns \
        -o jsonpath='{.data.NodeHosts}')" || {
        echo "❌ could not read CoreDNS's NodeHosts in ${CONTEXT}" >&2
        exit 1
    }

    if printf '%s\n' "${hosts}" | awk '{ for (i = 2; i <= NF; i++) if ($i == "host.k3d.internal") found = 1 } END { exit !found }'; then
        echo "✅ host.k3d.internal is in CoreDNS's NodeHosts"
        return
    fi

    ip="$(host_gateway_ip)"
    if [ -z "${ip}" ]; then
        echo "❌ could not tell the host's address on the k3d-${CLUSTER_NAME} network" >&2
        exit 1
    fi

    hosts="$(printf '%s\n' "${hosts}" | sed '/^[[:space:]]*$/d')"
    if [ -n "${hosts}" ]; then
        hosts="${hosts}"$'\n'
    fi
    hosts="${hosts}${ip} host.k3d.internal"
    patch="$(jq -n --arg hosts "${hosts}" '{data: {NodeHosts: $hosts}}')"

    kubectl --context "${CONTEXT}" -n kube-system patch configmap coredns \
        --type merge -p "${patch}" >/dev/null
    # CoreDNS reloads the file on its own, but only once the kubelet has synced
    # the mounted ConfigMap, which can take a minute. A restart is immediate.
    kubectl --context "${CONTEXT}" -n kube-system rollout restart deployment coredns >/dev/null
    kubectl --context "${CONTEXT}" -n kube-system rollout status deployment coredns \
        --timeout=2m >/dev/null

    echo "🔧 host.k3d.internal was missing from CoreDNS: added as ${ip}"
}

down() {
    require k3d
    echo "🗑️  deleting cluster ${CLUSTER_NAME}"
    k3d cluster delete "${CLUSTER_NAME}"
}

status() {
    require kubectl
    if ! kubectl config get-contexts "${CONTEXT}" >/dev/null 2>&1; then
        echo "❌ cluster ${CLUSTER_NAME} does not exist — run: make local-up"
        exit 1
    fi

    echo "context: ${CONTEXT}"
    echo
    echo "CloudNativePG:"
    kubectl --context "${CONTEXT}" -n cnpg-system get deployment cnpg-controller-manager \
        --no-headers 2>/dev/null || echo "  not installed"
    echo
    echo "Autharie CRDs:"
    kubectl --context "${CONTEXT}" get crd -o name 2>/dev/null | grep autharie || echo "  none"
    echo
    echo "Envoy Gateway:"
    kubectl --context "${CONTEXT}" -n envoy-gateway-system get deployment envoy-gateway \
        --no-headers 2>/dev/null || echo "  not installed"
    echo
    echo "KEDA:"
    kubectl --context "${CONTEXT}" -n keda get deployment keda-operator \
        --no-headers 2>/dev/null || echo "  not installed"
    echo
    echo "GatewayClass:"
    kubectl --context "${CONTEXT}" get gatewayclass --no-headers 2>/dev/null || echo "  none"
    echo
    echo "Gateways:"
    kubectl --context "${CONTEXT}" get gateway -A --no-headers 2>/dev/null || echo "  none"
    echo
    echo "IngressClass:"
    kubectl --context "${CONTEXT}" get ingressclass --no-headers 2>/dev/null || echo "  none"
}

case "${1:-}" in
    up) up ;;
    down) down ;;
    status) status ;;
    host-entry) host_entry ;;
    *) usage; exit 1 ;;
esac
