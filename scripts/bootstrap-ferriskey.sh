#!/usr/bin/env bash
# Applies deploy/ferriskey/terraform to a local FerrisKey and prints the values
# the control plane, Herald and the console need.
#
# The realm is declared with FerrisKey's own Terraform provider rather than
# driven through the API by a script: it stays in step with the API, it manages
# redirect URIs, and it converges. `ferris-ctl realm import` was the other
# candidate and does not converge -- it creates, and returns a 500 on a realm
# that already exists.
set -euo pipefail

FERRISKEY_URL="${FERRISKEY_URL:-http://localhost:3334}"
ADMIN_USER="${FERRISKEY_ADMIN_USER:-admin}"
ADMIN_PASSWORD="${FERRISKEY_ADMIN_PASSWORD:-admin}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TF_DIR="${REPO_ROOT}/deploy/ferriskey/terraform"

require() {
    command -v "$1" >/dev/null 2>&1 || { echo "❌ $1 is required" >&2; exit 1; }
}
require terraform
require curl
require jq

echo "🏛  applying ${TF_DIR}"
terraform -chdir="${TF_DIR}" init -input=false -no-color >/dev/null
TF_VAR_ferriskey_url="${FERRISKEY_URL}" \
TF_VAR_admin_username="${ADMIN_USER}" \
TF_VAR_admin_password="${ADMIN_PASSWORD}" \
    terraform -chdir="${TF_DIR}" apply -auto-approve -input=false -no-color \
    | tail -1

ISSUER=$(terraform -chdir="${TF_DIR}" output -raw issuer)
HERALD_UUID=$(terraform -chdir="${TF_DIR}" output -raw herald_client_uuid)
OPERATOR_UUID=$(terraform -chdir="${TF_DIR}" output -raw operator_client_uuid)
OPERATOR_ROLE=$(terraform -chdir="${TF_DIR}" output -raw operator_role_id)
GATEWAY_UUID=$(terraform -chdir="${TF_DIR}" output -raw mcp_gateway_client_uuid)
GATEWAY_ROLE=$(terraform -chdir="${TF_DIR}" output -raw mcp_gateway_role_id)
MCP_GATEWAY_RESOURCE="${MCP_GATEWAY_RESOURCE:-http://localhost:${MCP_GATEWAY_PORT:-9090}/mcp}"
API_AUDIENCE="${API_AUDIENCE:-autharie-realm}"

# The provider stores "***" for the secret at v0.1.0 -- the API masks it in the
# client response and the provider records the mask. So the real one is read
# from the endpoint that returns it. Drop this once the provider is fixed and
# take `terraform output -raw herald_client_secret` instead.
TOKEN=$(curl -sS -X POST \
    "${FERRISKEY_URL}/realms/master/protocol/openid-connect/token" \
    -d grant_type=password -d client_id=admin-cli \
    -d "username=${ADMIN_USER}" -d "password=${ADMIN_PASSWORD}" \
    | jq -r '.access_token')

if [ -z "${TOKEN}" ] || [ "${TOKEN}" = "null" ]; then
    echo "❌ could not authenticate as ${ADMIN_USER}." >&2
    exit 1
fi

# Checks the status, not curl's exit code: curl exits 0 on a 401 without -f,
# and trusting it once already turned a missing realm into a reported success.
client_secret() {
    local uuid="$1" name="$2" response status
    response=$(curl -sS "${FERRISKEY_URL}/realms/autharie/clients/${uuid}/client-secret" \
        -H "Authorization: Bearer ${TOKEN}" -w $'\n%{http_code}')
    status="${response##*$'\n'}"
    if [ "${status#2}" = "${status}" ]; then
        echo "❌ could not read the ${name} secret (HTTP ${status})" >&2
        exit 1
    fi
    printf '%s' "${response%$'\n'*}" | jq -r '.data.client_secret // .client_secret'
}

SECRET=$(client_secret "${HERALD_UUID}" herald-service)
OPERATOR_SECRET=$(client_secret "${OPERATOR_UUID}" autharie-operator-cli)

# The role has to be granted, not declared: it lives on the service account
# user behind the client, and the provider has no resource for that user at
# v0.1.0. Without it the client authenticates and is refused by every endpoint
# it exists to call.
operator_user=$(curl -sS "${FERRISKEY_URL}/realms/autharie/users" \
    -H "Authorization: Bearer ${TOKEN}" \
    | jq -r '.data[]? | select(.username == "service-account-autharie-operator-cli") | .id' \
    | head -1)

if [ -z "${operator_user}" ]; then
    echo "❌ the autharie-operator-cli service account is missing" >&2
    exit 1
fi

# Assigning a role the account already holds answers 200, so this converges
# rather than having to be guarded.
curl -sS -o /dev/null -X POST \
    "${FERRISKEY_URL}/realms/autharie/users/${operator_user}/roles/${OPERATOR_ROLE}" \
    -H "Authorization: Bearer ${TOKEN}"

# ------------------------------------------------------------- MCP gateway
#
# What the provider cannot declare at v0.1.0 (see main.tf), through the admin
# API. Each call is a PUT or tolerates "already there", so this converges.

# Checks the status like client_secret above: curl exits 0 on a 4xx.
admin_call() {
    local what="$1" method="$2" path="$3" body="${4:-}" tolerate="${5:-}" response status
    response=$(curl -sS -X "${method}" "${FERRISKEY_URL}/realms/autharie${path}" \
        -H "Authorization: Bearer ${TOKEN}" -H 'Content-Type: application/json' \
        ${body:+-d "${body}"} -w $'\n%{http_code}')
    status="${response##*$'\n'}"
    if [ "${status#2}" != "${status}" ] || [ "${status}" = "${tolerate}" ]; then
        return 0
    fi
    echo "❌ ${what} failed (HTTP ${status}): ${response%$'\n'*}" >&2
    exit 1
}

GATEWAY_SECRET=$(client_secret "${GATEWAY_UUID}" mcp-gateway)

gateway_user=$(curl -sS "${FERRISKEY_URL}/realms/autharie/users" \
    -H "Authorization: Bearer ${TOKEN}" \
    | jq -r '.data[]? | select(.username == "service-account-mcp-gateway") | .id' \
    | head -1)
if [ -z "${gateway_user}" ]; then
    echo "❌ the mcp-gateway service account is missing" >&2
    exit 1
fi

# Lets the gateway ask the realm's decision point (EvaluateAuthorization).
# Asked first: assigning a role the account already holds is an error here.
gateway_holds=$(curl -sS "${FERRISKEY_URL}/realms/autharie/users/${gateway_user}/roles" \
    -H "Authorization: Bearer ${TOKEN}" \
    | jq -r '[.data[]?.name] | index("mcp-gateway") // empty')
if [ -z "${gateway_holds}" ]; then
    admin_call "granting the mcp-gateway role" POST "/users/${gateway_user}/roles/${GATEWAY_ROLE}"
fi

# Agents name the endpoint they call in the `resource` parameter and the realm
# puts it in the token's audience, but only for a resource it was told about.
# CIMD is how an agent registers itself without a pre-shared client.
admin_call "enabling CIMD and the allowed resource" PUT "/settings" \
    "$(jq -nc --arg r "${MCP_GATEWAY_RESOURCE}" '{cimd_enabled: true, allowed_resources: [$r]}')"

# A token for that audience is only exchangeable by its owner.
admin_call "registering the resource owner" PUT "/resource-owners" \
    "$(jq -nc --arg r "${MCP_GATEWAY_RESOURCE}" --arg c "${GATEWAY_UUID}" '{owners: [{uri: $r, client_id: $c}]}')"

# Exchanging a caller's token for one issued to the API, the agent in `act`:
# delegation only, never impersonation. The target is the realm's own client for
# the API, whose id is the audience the API accepts.
admin_call "enabling token exchange for mcp-gateway" PATCH "/clients/${GATEWAY_UUID}" \
    '{"token_exchange_enabled": true}'
admin_call "creating the exchange policy" POST "/clients/${GATEWAY_UUID}/token-exchange-policies" \
    "$(jq -nc --arg a "${API_AUDIENCE}" '{target_audience: $a, allow_delegation: true, allow_impersonation: false}')" 409

# The gateway's configuration, with its secret. Written beside the template and
# kept out of git; the running gateway picks it up within seconds.
MCP_GATEWAY_CONFIG_FILE="${REPO_ROOT}/deploy/local/mcp-gateway.toml"
CLIENT_SECRET="${GATEWAY_SECRET}" awk '{
    while ((i = index($0, "@CLIENT_SECRET@")) > 0)
        $0 = substr($0, 1, i - 1) ENVIRON["CLIENT_SECRET"] substr($0, i + 15)
    print
}' "${REPO_ROOT}/deploy/local/mcp-gateway.toml.tmpl" > "${MCP_GATEWAY_CONFIG_FILE}.new"
mv "${MCP_GATEWAY_CONFIG_FILE}.new" "${MCP_GATEWAY_CONFIG_FILE}"

# Anybody carrying the operator realm role, so whoever calls this can hand
# them the platform rights that role used to imply. Resolved here because
# listing a realm's users needs the admin token, which lives in this script and
# nowhere else -- the operator client is refused, and rightly.
#
# Reported rather than acted on: what somebody may do to the control plane is
# the control plane's to record, and this script does not talk to it.
people=$(curl -sS "${FERRISKEY_URL}/realms/autharie/users" \
    -H "Authorization: Bearer ${TOKEN}" 2>/dev/null \
    | jq -r '.data[]? | select(.username | startswith("service-account-") | not) | .id' 2>/dev/null || true)

OPERATOR_PEOPLE=""
for person in ${people}; do
    holds=$(curl -sS "${FERRISKEY_URL}/realms/autharie/users/${person}/roles" \
        -H "Authorization: Bearer ${TOKEN}" 2>/dev/null \
        | jq -r '[.data[]?.name] | index("autharie-operator") // empty' 2>/dev/null || true)
    [ -n "${holds}" ] && OPERATOR_PEOPLE="${OPERATOR_PEOPLE}${OPERATOR_PEOPLE:+,}${person}"
done

cat <<SUMMARY

✅ realm ready

   Control plane:
     AUTH_ISSUER=${ISSUER}

   Herald:
     AUTH_ISSUER=${ISSUER}
     AUTH_CLIENT_ID=herald-service
     AUTH_CLIENT_SECRET=${SECRET}

   Operating the installation (registering data planes, publishing releases):
     OPERATOR_CLIENT_ID=autharie-operator-cli
     OPERATOR_CLIENT_SECRET=${OPERATOR_SECRET}
     OPERATOR_SUBJECT=${operator_user}
     OPERATOR_PEOPLE=${OPERATOR_PEOPLE}

   MCP gateway (agents), written to deploy/local/mcp-gateway.toml:
     endpoint ${MCP_GATEWAY_RESOURCE}
     client mcp-gateway, exchanges for ${API_AUDIENCE}

   Console (apps/console/.env):
     VITE_OIDC_ISSUER_URL=${ISSUER}
     VITE_OIDC_CLIENT_ID=console

   The secret is printed rather than written: it belongs in whatever holds your
   secrets, not in the working tree.

   Your own account needs a platform right to see the operator screens. The
   control plane holds those, not this realm: set AUTHARIE_BOOTSTRAP_OPERATOR to
   your subject, or have an operator grant you:
     curl -X PUT "<control plane>/platform/operators/<your-subject>" \\
          -H "Authorization: Bearer <an operator's token>" \\
          -d '{"rights":["view_estate"]}'

   Preview a change to the realm:
     terraform -chdir=deploy/ferriskey/terraform plan
SUMMARY
