#!/usr/bin/env bash
# Creates an account in the Autharie realm for one person and prints a
# temporary password, shown once. FerrisKey asks for a new one at the first
# sign-in.
#
# This is how accounts are made while self-registration is off (the beta runs
# on an access list): the realm setting `user_registration_enabled` stays
# false and nobody can create an account from the login page.
#
#   FERRISKEY_URL=https://auth.example/api \
#   FERRISKEY_ADMIN_USER=... FERRISKEY_ADMIN_PASSWORD=... \
#   scripts/invite-user.sh someone@example.com
set -euo pipefail

EMAIL="${1:?usage: $0 <email>}"
FERRISKEY_URL="${FERRISKEY_URL:-http://localhost:3334}"
REALM="${REALM:-autharie}"
ADMIN_USER="${FERRISKEY_ADMIN_USER:?FERRISKEY_ADMIN_USER is required}"
ADMIN_PASSWORD="${FERRISKEY_ADMIN_PASSWORD:?FERRISKEY_ADMIN_PASSWORD is required}"

require() {
    command -v "$1" >/dev/null 2>&1 || { echo "❌ $1 is required" >&2; exit 1; }
}
require curl
require jq
require openssl
require uuidgen

case "${EMAIL}" in
    *@*.*) ;;
    *) echo "❌ ${EMAIL} is not an email address" >&2; exit 1 ;;
esac

TOKEN=$(curl -sS -X POST \
    "${FERRISKEY_URL}/realms/master/protocol/openid-connect/token" \
    --data-urlencode grant_type=password --data-urlencode client_id=admin-cli \
    --data-urlencode "username=${ADMIN_USER}" --data-urlencode "password=${ADMIN_PASSWORD}" \
    | jq -r '.access_token')

if [ -z "${TOKEN}" ] || [ "${TOKEN}" = "null" ]; then
    echo "❌ could not authenticate as ${ADMIN_USER}." >&2
    exit 1
fi

# Checks the status, not curl's exit code: curl exits 0 on a 4xx without -f.
call() {
    local method="$1" path="$2" body="$3" response status
    response=$(curl -sS -X "${method}" "${FERRISKEY_URL}/realms/${REALM}${path}" \
        -H "Authorization: Bearer ${TOKEN}" -H 'Content-Type: application/json' \
        -d "${body}" -w $'\n%{http_code}')
    status="${response##*$'\n'}"
    if [ "${status#2}" = "${status}" ]; then
        echo "❌ ${method} ${path} answered ${status}" >&2
        exit 1
    fi
}

USER_ID=$(uuidgen | tr '[:upper:]' '[:lower:]')
call POST /users "$(jq -n --arg id "${USER_ID}" --arg email "${EMAIL}" \
    '{id: $id, username: $email, email: $email, email_verified: false}')"

PASSWORD="$(openssl rand -base64 18 | tr -d '/+=')Aa1!"
call PUT "/users/${USER_ID}/reset-password" "$(jq -n --arg value "${PASSWORD}" \
    '{credential_type: "password", value: $value, temporary: true}')"

echo "✅ ${EMAIL} can sign in once with this temporary password:"
echo "${PASSWORD}"
