#!/usr/bin/env bash
# Brings the whole of Autharie up locally and leaves you able to create a
# deployment from the console.
#
# There are two runtimes and the split is the architecture, not an accident:
# the control plane runs in Docker Compose, the data plane in a k3d cluster,
# and the data plane *pulls* its work. Nothing here ever gives the control
# plane credentials to the cluster.
#
# Every step is idempotent. Run it again after a failure rather than starting
# from a clean machine.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

# Ports are overridable because a developer machine already has things on the
# usual ones. Every default here has collided with something real at least once.
export AUTHARIE_POSTGRES_PORT="${AUTHARIE_POSTGRES_PORT:-55435}"
export AUTHARIE_API_PORT="${AUTHARIE_API_PORT:-7777}"
export FERRISKEY_API_PORT="${FERRISKEY_API_PORT:-3334}"
export FERRISKEY_WEBAPP_PORT="${FERRISKEY_WEBAPP_PORT:-5556}"
# The MCP gateway in front of the Autharie MCP server. The realm's allowed
# resource is derived from it (bootstrap-ferriskey.sh), so the two must agree.
export MCP_GATEWAY_PORT="${MCP_GATEWAY_PORT:-9090}"
# The published port, because the data plane reaches RustFS from outside the
# compose network. The control plane uses the container port on the inside; the
# two are the same store and the endpoints are deliberately not.
RUSTFS_PORT="${RUSTFS_PORT:-9800}"
# Same reasoning as RUSTFS_PORT above: Herald runs inside the k3d cluster and
# reaches this Quickwit -- started by the compose stack below, no profile of
# its own -- from outside the compose network.
QUICKWIT_PORT="${QUICKWIT_PORT:-7280}"
# Matches local-cluster.sh's own default: the host port its k3d load balancer
# maps to the Gateway's HTTPS listener, since a container cannot bind the
# real 443. Every provisioned instance's webapp/API URLs need to know it, or
# a browser redirect assumes 443 and cannot connect.
export AUTHARIE_HTTPS_PORT="${AUTHARIE_HTTPS_PORT:-8444}"

CONSOLE_PORT="${CONSOLE_PORT:-5173}"
CONTROL_PLANE="http://localhost:${AUTHARIE_API_PORT}"
FERRISKEY_URL="http://localhost:${FERRISKEY_API_PORT}"
REGION="${AUTHARIE_REGION:-local}"
CLUSTER="${AUTHARIE_CLUSTER:-autharie-local}"
RELEASE="${AUTHARIE_RELEASE:-autharie-dataplane}"
NAMESPACE="${AUTHARIE_DATAPLANE_NAMESPACE:-autharie-system}"

step() { printf '\n\033[1;35m▸ %s\033[0m\n' "$1"; }
note() { printf '  %s\n' "$1"; }
die()  { printf '\n\033[1;31m✗ %s\033[0m\n' "$1" >&2; exit 1; }
warn() { printf '\n\033[1;33m! %s\033[0m\n' "$1"; }

for tool in docker k3d helm kubectl terraform jq curl; do
    command -v "${tool}" >/dev/null 2>&1 || die "${tool} is required"
done

# Tearing down deletes a cluster and a database, so it is a named argument
# rather than a flag that could be reached by a typo.
if [ "${1:-up}" = "down" ]; then
    step "tearing down"
    docker compose --profile ferriskey down --volumes 2>&1 | tail -2 || true
    ./scripts/local-cluster.sh down || true
    note "the terraform state in deploy/ferriskey/terraform is left alone:"
    note "it describes a realm whose database has just been deleted, so run"
    note "  terraform -chdir=deploy/ferriskey/terraform state rm ferriskey_realm.autharie"
    note "if the next run complains that the realm already exists."
    exit 0
fi

if [ "${1:-up}" != "up" ]; then
    die "usage: demo.sh [up|down]"
fi

# ---------------------------------------------------------------- control plane

# Whoever operates this installation, before anybody has been granted it.
# Platform rights are handed out by somebody who already holds one, and a fresh
# database has nobody to hand out the first. Named here rather than left to a
# realm role: the realm says who somebody is, the control plane says what they
# may do to it, and this is the seam between the two.
#
# Resolved below, from the realm, once the realm exists -- so the first Compose
# start of a fresh checkout comes up with nobody, says so in its logs, and is
# granted on the second pass a few lines later.
export AUTHARIE_BOOTSTRAP_OPERATOR="${AUTHARIE_BOOTSTRAP_OPERATOR:-}"
export CUSTOMER_CLOUD_ENABLED="${CUSTOMER_CLOUD_ENABLED:-false}"
export CUSTOMER_CLOUD_CONTROL_PLANE_URL="${CUSTOMER_CLOUD_CONTROL_PLANE_URL:-}"
export CUSTOMER_CLOUD_HERALD_ISSUER="${CUSTOMER_CLOUD_HERALD_ISSUER:-}"

# Deployments are served at <name>.<zone>, and the control plane decides those
# names from OVH_ZONE. It *publishes* a record for them only when the OVH
# credentials are set as well -- and a developer's .env may carry the real zone
# and real credentials. A demo that did that would leave public records for
# throwaway deployments in a production zone. So by default the demo
# runs on a zone of its own with the credentials blanked: in Compose's
# substitution the shell's environment wins over .env, empty values included.
# *.autharie.local is also exactly what the mkcert certificate below covers.
#
# DEMO_PUBLISH_DNS=1 keeps whatever .env says, publishing included.
DEMO_ZONE="${DEMO_ZONE:-autharie.local}"
DEMO_PUBLISH_DNS="${DEMO_PUBLISH_DNS:-0}"
if [ "${DEMO_PUBLISH_DNS}" = "1" ]; then
    # Read through Compose, so .env and the environment combine exactly as they
    # will for the container. Only the zone comes out; the credentials are
    # checked for presence inside jq and never printed.
    TENANT_ZONE=$(docker compose --profile ferriskey config --format json 2>/dev/null \
        | jq -r '.services.autharie.environment
            | if ([.OVH_APPLICATION_KEY, .OVH_APPLICATION_SECRET, .OVH_CONSUMER_KEY]
                  | all(. != null and . != ""))
              then (.OVH_ZONE // "") else "" end') || TENANT_ZONE=""
    [ -n "${TENANT_ZONE}" ] \
        || die "DEMO_PUBLISH_DNS=1 needs OVH_ZONE and the three OVH credentials, in .env or the environment"
    DNS_RECORDS="published in ${TENANT_ZONE}"
else
    export OVH_ZONE="${DEMO_ZONE}"
    export OVH_APPLICATION_KEY="" OVH_APPLICATION_SECRET="" OVH_CONSUMER_KEY=""
    TENANT_ZONE="${DEMO_ZONE}"
    DNS_RECORDS="not published (DEMO_PUBLISH_DNS=1 to publish)"
fi

# Deployments are served by the k3d gateway on this machine, but inside the
# control plane's container their name resolves to the container itself (the
# /etc/hosts entry leaks in), so the availability probe would call itself and
# report every deployment down. It is pointed at the host's published HTTPS
# port instead and told to trust the mkcert CA that signed the certificate.
# Not with DEMO_PUBLISH_DNS=1: that zone is real and resolves for real.
# mkcert -install is what creates the CA on a fresh machine; it is idempotent.
if [ "${DEMO_PUBLISH_DNS}" != "1" ]; then
    command -v mkcert >/dev/null 2>&1 || die "mkcert is required (brew install mkcert)"
    mkcert -install >/dev/null 2>&1
    MKCERT_CAROOT="$(mkcert -CAROOT)"
    [ -f "${MKCERT_CAROOT}/rootCA.pem" ] \
        || die "no mkcert CA at ${MKCERT_CAROOT}/rootCA.pem (run: mkcert -install)"
    export AUTHARIE_LOCAL_CA="${MKCERT_CAROOT}/rootCA.pem"
    export REACHABILITY_PROBE_VIA="host.docker.internal:${AUTHARIE_HTTPS_PORT}"
fi

# Recreated rather than restarted: the control plane reads its configuration
# from the environment at startup, and a restart keeps the environment it had.
# Everything exported above -- ports, bootstrap operator, zone -- goes with it.
recreate_control_plane() {
    docker compose --profile ferriskey up -d --force-recreate --no-deps autharie >/dev/null 2>&1 \
        || die "$1"
    for _ in $(seq 30); do
        status=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 \
            "${CONTROL_PLANE}/swagger" 2>/dev/null) || status=000
        [ "${status}" != "000" ] && break
        sleep 2
    done
}

step "control plane (docker compose)"
docker compose --profile ferriskey up -d --build --wait 2>&1 | tail -3 \
    || die "compose failed to come up"

# `--wait` returns when containers are healthy, which is not the same as the
# API serving: the control plane binds its port after connecting to Postgres.
note "waiting for ${CONTROL_PLANE}"
for _ in $(seq 60); do
    status=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 2 \
        "${CONTROL_PLANE}/swagger" 2>/dev/null) || status=000
    [ "${status}" != "000" ] && break
    sleep 2
done
[ "${status}" != "000" ] || die "the control plane never answered on ${CONTROL_PLANE}"
note "control plane is answering"
note "tenant zone ${TENANT_ZONE}, DNS records ${DNS_RECORDS}"

# -------------------------------------------------------------------- identity

step "identity (terraform against FerrisKey)"
# The realm only allows the redirect URIs it was told about, so the console's
# port has to be decided here rather than by whichever one Vite happens to find
# free. Getting this wrong fails at the identity provider, with an error that
# says nothing about a port.
export TF_VAR_console_redirect_uris="[\"http://localhost:${CONSOLE_PORT}\",\"http://localhost:${CONSOLE_PORT}/*\",\"http://localhost:${FERRISKEY_WEBAPP_PORT}\",\"http://localhost:${FERRISKEY_WEBAPP_PORT}/*\"]"
bootstrap=$(FERRISKEY_URL="${FERRISKEY_URL}" ./scripts/bootstrap-ferriskey.sh)
ISSUER=$(printf '%s' "${bootstrap}" | awk -F= '/^ *AUTH_ISSUER=/{print $2; exit}')
OPERATOR_SUBJECT=$(printf '%s' "${bootstrap}" | awk -F= '/^ *OPERATOR_SUBJECT=/{print $2; exit}')
OPERATOR_PEOPLE=$(printf '%s' "${bootstrap}" | awk -F= '/^ *OPERATOR_PEOPLE=/{print $2; exit}')
HERALD_SECRET_SHARED=$(printf '%s' "${bootstrap}" | awk -F= '/^ *AUTH_CLIENT_SECRET=/{print $2; exit}')
HERALD_SECRET="${HERALD_SECRET_SHARED}"
OPERATOR_SECRET=$(printf '%s' "${bootstrap}" | awk -F= '/^ *OPERATOR_CLIENT_SECRET=/{print $2; exit}')
if [ -z "${ISSUER}" ] || [ -z "${HERALD_SECRET}" ] || [ -z "${OPERATOR_SECRET}" ]; then
    die "could not read the realm bootstrap output"
fi
note "issuer ${ISSUER}"

token() {
    # Checked on status, not on curl's exit code: without -f, curl exits 0 on a
    # 401, and a script that trusts it reports success against a realm that
    # rejected it.
    local client="$1" secret="$2" response status
    response=$(curl -sS -X POST \
        "${FERRISKEY_URL}/realms/autharie/protocol/openid-connect/token" \
        -d grant_type=client_credentials \
        -d "client_id=${client}" \
        -d "client_secret=${secret}" \
        -w $'\n%{http_code}')
    status="${response##*$'\n'}"
    [ "${status#2}" != "${status}" ] || die "could not obtain a token for ${client} (HTTP ${status})"
    printf '%s' "${response%$'\n'*}" | jq -r '.access_token'
}

# Obtained and thrown away: what matters is that the secret the chart is about
# to receive actually authenticates, which is cheaper to find out here than
# from a Herald that comes up and quietly claims nothing.
token herald-service "${HERALD_SECRET_SHARED}" >/dev/null
# Registering a data plane is an operator's act, not a data plane's. Herald's
# token is refused by those endpoints, and rightly: a data plane that could
# register another one could point work at a cluster nobody chose.
OPERATOR_TOKEN=$(token autharie-operator-cli "${OPERATOR_SECRET}")
note "herald-service and autharie-operator-cli can authenticate"

# ------------------------------------------------------------ platform rights

step "granting the platform rights"
# The control plane holds these, not the realm. A run before this existed left
# an installation whose operator screens refused everybody, which reads as a
# broken console rather than as an empty table.
if [ -n "${OPERATOR_SUBJECT}" ] && [ "${AUTHARIE_BOOTSTRAP_OPERATOR}" != "${OPERATOR_SUBJECT}" ]; then
    export AUTHARIE_BOOTSTRAP_OPERATOR="${OPERATOR_SUBJECT}"
    recreate_control_plane "could not restart the control plane with a bootstrap operator"
    OPERATOR_TOKEN=$(token autharie-operator-cli "${OPERATOR_SECRET}")
fi
note "autharie-operator-cli operates this installation"

# Anybody still carrying the old realm role gets the rights it used to imply.
# That role was the authority until the control plane took it over; seeding
# from it is what stops this run locking out the account that could already see
# these screens.
if [ -z "${OPERATOR_PEOPLE}" ]; then
    note "nobody carries the autharie-operator realm role yet"
    note "grant yourself once you have logged in:"
    note "  curl -X PUT ${CONTROL_PLANE}/platform/operators/<your-subject> \\"
    note "       -H 'Authorization: Bearer <the operator cli token>' \\"
    note "       -H 'Content-Type: application/json' \\"
    note "       -d '{\"rights\":[\"view_estate\",\"operate_fleet\",\"act_on_tenant\",\"manage_operators\"]}'"
fi

for person in $(printf '%s' "${OPERATOR_PEOPLE}" | tr ',' ' '); do
    granted=$(curl -sS -o /dev/null -w '%{http_code}' -X PUT \
        "${CONTROL_PLANE}/platform/operators/${person}" \
        -H "Authorization: Bearer ${OPERATOR_TOKEN}" \
        -H 'Content-Type: application/json' \
        -d '{"rights":["view_estate","operate_fleet","act_on_tenant","manage_operators"]}')
    if [ "${granted#2}" != "${granted}" ]; then
        note "granted ${person}"
    else
        note "could not grant ${person} (HTTP ${granted})"
    fi
done

# ------------------------------------------------------------------- k3d cluster

step "data plane cluster (k3d)"
if k3d cluster list -o json | jq -e --arg n "${CLUSTER}" '.[] | select(.name == $n)' >/dev/null; then
    note "${CLUSTER} already exists"
    # A cluster made before the data plane needed KEDA has none, and the chart's
    # ScaledObject cannot be created without its CRD.
    if ! kubectl --context "k3d-${CLUSTER}" get crd scaledobjects.keda.sh >/dev/null 2>&1; then
        note "installing KEDA"
        helm --kube-context "k3d-${CLUSTER}" upgrade --install keda keda \
            --repo https://kedacore.github.io/charts \
            --version "${KEDA_VERSION:-2.21.0}" \
            --namespace keda --create-namespace \
            --wait --timeout 5m >/dev/null 2>&1 || die "could not install KEDA"
    fi
else
    ./scripts/local-cluster.sh up
fi
KUBECONFIG_PATH=$(k3d kubeconfig write "${CLUSTER}")
export KUBECONFIG="${KUBECONFIG_PATH}"
note "kubeconfig ${KUBECONFIG_PATH}"

# Every pod that talks to this machine does it as host.k3d.internal: Herald to
# the control plane, the issuer, Quickwit, RustFS. k3d's DNS entry for it does
# not survive a Docker restart, and a Herald that cannot resolve it simply goes
# quiet -- so it is put back if missing, then proven with a real request rather
# than trusted.
entry=$(AUTHARIE_CLUSTER_NAME="${CLUSTER}" ./scripts/local-cluster.sh host-entry 2>&1) \
    || die "could not check host.k3d.internal in CoreDNS: ${entry}"
note "${entry}"

kubectl -n default delete pod autharie-reachability --ignore-not-found --wait >/dev/null 2>&1 || true
reached=$(kubectl -n default run autharie-reachability --rm -i --quiet --restart=Never \
    --image=curlimages/curl:8.11.1 --pod-running-timeout=2m --command -- \
    curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    "http://host.k3d.internal:${AUTHARIE_API_PORT}/swagger" 2>/dev/null) || true
reached=$(printf '%s' "${reached}" | grep -oE '[0-9]{3}' | tail -1) || true
if [ -z "${reached}" ] || [ "${reached}" = "000" ]; then
    die "pods in ${CLUSTER} cannot reach the control plane at http://host.k3d.internal:${AUTHARIE_API_PORT}.
  Check the entry:  kubectl -n kube-system get configmap coredns -o jsonpath='{.data.NodeHosts}'
  It must point at this machine as seen from the k3d-${CLUSTER} network."
fi
note "pods reach the control plane at host.k3d.internal (HTTP ${reached})"

# ------------------------------------------------------------- data plane images

step "building the data plane images"
# Built here and imported into k3d rather than pulled from ghcr, for one reason
# that matters: a demo that pulls :latest tests published code, not the branch
# you have checked out. It also removes a registry -- and a login -- from the
# path of getting the thing running on a laptop.
#
# The layers are shared with the control-plane build above, so this is much
# cheaper than five Rust builds after the first run.
IMAGE_REGISTRY="autharie.local"
IMAGE_REPO="demo"
IMAGE_TAG="dev"

BUILD_LOG="$(mktemp -t autharie-demo-build)"
for component in herald genesis operator mcp-gateway ferriskey-mcp; do
    image="${IMAGE_REGISTRY}/${IMAGE_REPO}/autharie-${component}:${IMAGE_TAG}"
    note "building ${component}"
    # BuildKit writes its progress to stderr, so `>/dev/null` alone leaves a
    # thousand lines of cargo output in whatever is capturing this. Kept in a
    # file and printed only if the build fails, which is the only time anyone
    # wants it.
    if ! docker build --target "${component}" -t "${image}" . >"${BUILD_LOG}" 2>&1; then
        tail -30 "${BUILD_LOG}" >&2
        die "could not build the ${component} image (full log: ${BUILD_LOG})"
    fi
    k3d image import "${image}" --cluster "${CLUSTER}" >/dev/null 2>&1 \
        || die "could not import ${image} into ${CLUSTER}"
done
rm -f "${BUILD_LOG}"
note "five images imported into ${CLUSTER}"

# Kubernetes restarts a pod when its *spec* changes, and the tag does not move
# between runs. So a second run rebuilt three images, imported them, and left
# every pod on the binary from the first -- which reads as code that did not
# take effect. The restart below is what makes re-running this a redeploy.
RESTART_AFTER_INSTALL=1

# --------------------------------------------------------------- register the DP

step "registering the shared data plane"
# The token from the rights step has outlived its five minutes by now: building
# and importing the images takes longer than that on a cold cache.
OPERATOR_TOKEN=$(token autharie-operator-cli "${OPERATOR_SECRET}")
# Idempotent by lookup rather than by an upsert the API does not offer: running
# this twice must not leave two data planes competing for the same cluster.
existing=$(curl -sS "${CONTROL_PLANE}/dataplanes" -H "Authorization: Bearer ${OPERATOR_TOKEN}" \
    | jq -r --arg r "${REGION}" \
        'first(.data[] | select(.region == $r and .allocation == "shared") | .id) // empty')

if [ -n "${existing}" ]; then
    DATAPLANE_ID="${existing}"
    note "reusing ${DATAPLANE_ID}"

    # Its secret is only readable in the answer that created it, and this run
    # did not create it. Issued again rather than looked up -- which is also
    # what an operator does when a cluster's secret leaks.
    response=$(curl -sS -X POST "${CONTROL_PLANE}/dataplanes/${DATAPLANE_ID}/credential" \
        -H "Authorization: Bearer ${OPERATOR_TOKEN}" -w $'\n%{http_code}')
    status="${response##*$'\n'}"
    [ "${status#2}" != "${status}" ] || die "could not issue a credential (HTTP ${status})"
    HERALD_CLIENT_ID=$(printf '%s' "${response%$'\n'*}" | jq -r '.herald_client_id // empty')
    HERALD_SECRET=$(printf '%s' "${response%$'\n'*}" | jq -r '.herald_secret // empty')
else
    response=$(curl -sS -X POST "${CONTROL_PLANE}/dataplanes" \
        -H "Authorization: Bearer ${OPERATOR_TOKEN}" \
        -H 'Content-Type: application/json' \
        -d "{\"mode\":\"shared\",\"region\":\"${REGION}\",\"capacity\":{\"cpu_millis\":8000,\"memory_mib\":16384,\"storage_gib\":200}}" \
        -w $'\n%{http_code}')
    status="${response##*$'\n'}"
    [ "${status#2}" != "${status}" ] || die "could not register a data plane (HTTP ${status})"
    DATAPLANE_ID=$(printf '%s' "${response%$'\n'*}" | jq -r '.id')
    HERALD_CLIENT_ID=$(printf '%s' "${response%$'\n'*}" | jq -r '.herald_client_id // empty')
    HERALD_SECRET=$(printf '%s' "${response%$'\n'*}" | jq -r '.herald_secret // empty')
    note "registered ${DATAPLANE_ID}"
fi

if [ -n "${HERALD_CLIENT_ID}" ]; then
    note "this cluster authenticates as ${HERALD_CLIENT_ID}"
else
    # An installation with no realm administrator configured mints nothing, and
    # the chart falls back to the client every cluster shares.
    note "no identity of its own: falling back to herald-service"
    HERALD_CLIENT_ID="herald-service"
    HERALD_SECRET="${HERALD_SECRET_SHARED}"
fi

# It starts in Provisioning and becomes Active on its first heartbeat, which
# Herald sends once the chart below is running. Nothing here forces it.

# ------------------------------------------------------------------- helm install

step "installing the data plane into ${CLUSTER}"
# A second release in the same namespace would run a second Herald claiming for
# a different data plane id, against the same cluster. Both would appear to
# work and each would see half the actions. Left to the operator to remove
# rather than deleted here -- this script does not uninstall things it did not
# install.
others=$(helm -n "${NAMESPACE}" list -q 2>/dev/null | grep -v "^${RELEASE}$" || true)
if [ -n "${others}" ]; then
    printf '\n\033[1;33m! another data plane release is installed in %s:\033[0m\n' "${NAMESPACE}"
    printf '%s\n' "${others}" | sed 's/^/    /'
    note "its Herald claims actions for whatever id it was installed with."
    note "Remove it unless you meant it:  helm -n ${NAMESPACE} uninstall <name>"
fi

# The chart does not create this one: the credentials are an installation's,
# not a chart's. Without it the operator refuses every instance that archives,
# saying this data plane has no object store credentials -- which is true, and
# is the whole difference between a demo that backs up and one that does not.
kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl -n "${NAMESPACE}" create secret generic autharie-object-store \
    --from-literal=ACCESS_KEY_ID=autharie \
    --from-literal=ACCESS_SECRET_KEY=autharieautharie \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
note "object store credentials in place"

# ------------------------------------------------------------------- local TLS

step "local TLS (mkcert)"
# A browser treats plain HTTP on anything but localhost/127.0.0.1 as an
# insecure context, which is what silently disables things like
# crypto.randomUUID -- a real, trusted certificate is what makes an instance
# behave in a laptop's browser the way it will in production's.
command -v mkcert >/dev/null 2>&1 || die "mkcert is required (brew install mkcert)"
mkcert -install >/dev/null 2>&1
CERT_DIR="$(mktemp -d)"
mkcert -cert-file "${CERT_DIR}/tls.crt" -key-file "${CERT_DIR}/tls.key" \
    "autharie.local" "*.autharie.local" >/dev/null 2>&1 \
    || die "mkcert could not issue a certificate for *.autharie.local"
kubectl create namespace "${NAMESPACE}" --dry-run=client -o yaml | kubectl apply -f - >/dev/null
kubectl -n "${NAMESPACE}" create secret tls autharie-gateway-tls \
    --cert="${CERT_DIR}/tls.crt" --key="${CERT_DIR}/tls.key" \
    --dry-run=client -o yaml | kubectl apply -f - >/dev/null
rm -rf "${CERT_DIR}"
note "local CA trusted, *.autharie.local certificate installed"

helm upgrade --install "${RELEASE}" charts/autharie-dataplane \
    --namespace "${NAMESPACE}" --create-namespace \
    --set "objectStore.enabled=true" \
    --set "objectStore.endpoint=http://host.k3d.internal:${RUSTFS_PORT}" \
    --set "image.registry=${IMAGE_REGISTRY}" \
    --set "image.repository=${IMAGE_REPO}" \
    --set "image.tag=${IMAGE_TAG}" \
    --set "image.pullPolicy=Never" \
    --set "dataplane.id=${DATAPLANE_ID}" \
    --set "controlPlane.url=http://host.k3d.internal:${AUTHARIE_API_PORT}" \
    --set "controlPlane.auth.issuer=http://host.k3d.internal:${FERRISKEY_API_PORT}/realms/autharie" \
    --set "controlPlane.auth.clientId=${HERALD_CLIENT_ID}" \
    --set "controlPlane.auth.clientSecret=${HERALD_SECRET}" \
    --set "herald.logIndex.url=http://host.k3d.internal:${QUICKWIT_PORT}" \
    --set "herald.otlp.enabled=true" \
    --set "gateway.tls.secretName=autharie-gateway-tls" \
    --set "gateway.publicHttpsPort=${AUTHARIE_HTTPS_PORT}" \
    --set "agents.enabled=true" \
    --wait --timeout 5m 2>&1 | tail -4 || die "helm install failed"

# ------------------------------------------------------------------- new images

if [ "${RESTART_AFTER_INSTALL:-0}" = "1" ]; then
    step "restarting the data plane onto the images just built"
    kubectl -n "${NAMESPACE}" rollout restart deployment \
        -l app.kubernetes.io/part-of=autharie >/dev/null
    kubectl -n "${NAMESPACE}" rollout status deployment \
        -l app.kubernetes.io/part-of=autharie --timeout=3m >/dev/null \
        || note "some pods are still coming up; see kubectl -n ${NAMESPACE} get pods"
    note "running the code in this working tree"
fi

# ------------------------------------------------------------------ wait for life

step "waiting for the data plane to report"
# Freshly obtained. The token taken before the images were built is minutes old
# by now, and reading `.status` off a 401 body printed "the data plane is still
# 401" over a stack that was working -- a false alarm at the exact moment
# somebody trusts this script.
OPERATOR_TOKEN=$(token autharie-operator-cli "${OPERATOR_SECRET}")

# The heartbeat is what promotes it out of Provisioning: it is the only evidence
# the control plane gets that Herald is running inside the cluster.
for _ in $(seq 60); do
    status=$(curl -sS "${CONTROL_PLANE}/dataplanes/${DATAPLANE_ID}" \
        -H "Authorization: Bearer ${OPERATOR_TOKEN}" | jq -r '.data.status // .status')
    [ "${status}" = "active" ] && break
    sleep 5
done

if [ "${status}" != "active" ]; then
    printf '\n\033[1;33m! the data plane is still %s\033[0m\n' "${status}"
    note "Herald has not reported yet. Its logs:"
    note "  KUBECONFIG=${KUBECONFIG_PATH} kubectl -n ${NAMESPACE} logs -l app.kubernetes.io/component=herald --tail=50"
    note "Everything else is up; a deployment created now will sit in Pending."
fi

cat <<SUMMARY

$( [ "${status}" = "active" ] && printf '\033[1;32m✅ ready\033[0m' || printf '\033[1;33m⚠ up, data plane not reporting\033[0m' )

   data plane   ${DATAPLANE_ID}  (${REGION}, ${status})
   control API  ${CONTROL_PLANE}
   identity     ${ISSUER}
   tenant zone  ${TENANT_ZONE}  (DNS records ${DNS_RECORDS})

   Start the console:

     cd apps/console
     printf 'VITE_API_URL=%s\nVITE_OIDC_ISSUER_URL=%s\nVITE_OIDC_CLIENT_ID=console\n' \\
       '${CONTROL_PLANE}' '${ISSUER}' > .env
     pnpm install && pnpm dev -- --port ${CONSOLE_PORT} --host 127.0.0.1

   Open http://localhost:${CONSOLE_PORT} and **create an account** on the login page.

   The port matters: it is the one the realm allows a redirect to. 'pnpm dev'
   uses --strictPort so it fails rather than quietly moving to the next free
   port, which would fail later at the identity provider instead. If ${CONSOLE_PORT}
   is taken, re-run with CONSOLE_PORT=5175 and the realm follows.

   Self-registration is on for this realm, and it is the only way in: FerrisKey
   has no admin API for setting another user's password, so an account created
   by Terraform would exist and be unable to log in. Turned off for anything
   that is not a throwaway stack.

   Then create a deployment and watch it land:

     Data planes -> ${DATAPLANE_ID}

   A shared deployment is placed on the data plane above. A dedicated one
   provisions a data plane of its own for the organisation -- against the same
   k3d cluster locally, which is what the local provisioner is for.

   Once a FerrisKey deployment is Running and you have sent it some traffic,
   its logs and traces are searchable from the deployment's own Logs and
   Traces tabs -- Herald ships both to the Quickwit this script just pointed
   it at (http://host.k3d.internal:${QUICKWIT_PORT} from inside the cluster,
   http://localhost:${QUICKWIT_PORT} from here).

   Free deployments: in the console, create a deployment with the Free offer.
   It is a FerrisKey instance of its own, with its database on the data
   plane's shared database cluster, served at <name>.${TENANT_ZONE}. Make the
   hostname resolve, then open it on port ${AUTHARIE_HTTPS_PORT}:

     sudo make local-hosts-apply

   A FerrisKey deployment is built from the newest FerrisKey release marked
   available. The catalogue starts empty, so to use an image built locally,
   import it and publish it:

     k3d image import ghcr.io/ferriskey/ferriskey-api:<v> \\
       ghcr.io/ferriskey/ferriskey-webapp:<v> --cluster ${CLUSTER}
     curl -X POST ${CONTROL_PLANE}/releases/operator/ferriskey \\
       -H 'Authorization: Bearer <the operator cli token>' \\
       -H 'Content-Type: application/json' -d '{"version":"<v>","risk":"none"}'
     curl -X PUT ${CONTROL_PLANE}/releases/operator/ferriskey/<v>/status \\
       -H 'Authorization: Bearer <the operator cli token>' \\
       -H 'Content-Type: application/json' -d '{"status":"available"}'

   Agents (MCP): the Autharie MCP endpoint is http://localhost:${MCP_GATEWAY_PORT}/mcp.
   Without a token it answers 401 and names its metadata; with one issued by
   ${ISSUER} for that resource, the tools the realm's policies allow. The
   data plane runs a gateway and a FerrisKey MCP server of its own
   (agents.enabled=true); the operator wires a deployment to them.

   Tear down:  ./scripts/demo.sh down

SUMMARY
