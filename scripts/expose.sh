#!/usr/bin/env bash
# Puts the local control plane and the realm's token endpoint behind one public
# URL, so a cluster created in somebody else's cloud can reach them.
#
# A Herald pulls its work from the control plane and gets its token from the
# realm, so both have to be reachable from the internet. Exposing the realm
# whole would expose its master administrator, which locally is admin/admin: a
# small proxy in front lets through the token route of the autharie realm and
# nothing else of it. Everything else goes to the control plane, which wants a
# bearer token on all of it.
#
# One host serves both, which is what lets a tunnel with no domain do the job:
#   control plane  https://<host>
#   token issuer   https://<host>/realms/autharie
#
# The URL of a quick tunnel changes every time it starts, so the control plane
# has to be started again with the new one; `make demo` does that.
set -euo pipefail

EDGE_NAME="autharie-edge"
EDGE_IMAGE="nginx:1.28.0-alpine3.21-slim"
EDGE_PORT="${EXPOSE_EDGE_PORT:-8088}"
API_PORT="${AUTHARIE_API_PORT:-7777}"
IDP_PORT="${FERRISKEY_API_PORT:-3334}"
REALM="${EXPOSE_REALM:-autharie}"
TUNNEL="${EXPOSE_WITH:-cloudflared}"
STATE_DIR="${TMPDIR:-/tmp}/autharie-expose"
URL_FILE="${STATE_DIR}/url"
PID_FILE="${STATE_DIR}/tunnel.pid"
LOG_FILE="${STATE_DIR}/tunnel.log"

die() {
    echo "❌ $*" >&2
    exit 1
}

usage() {
    cat <<USAGE
usage: $(basename "$0") <up|down|env|status>

  up      start the proxy and a public tunnel, and print what to start the
          control plane with
  down    stop both
  env     print the variables again, for \`eval\`
  status  say whether the tunnel is up and where

EXPOSE_WITH=cloudflared (default) or ngrok chooses the tunnel.
USAGE
}

require() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

public_url() {
    [ -s "${URL_FILE}" ] && cat "${URL_FILE}"
}

tunnel_alive() {
    [ -s "${PID_FILE}" ] && kill -0 "$(cat "${PID_FILE}")" 2>/dev/null
}

print_env() {
    local url
    url="$(public_url)" || die "no tunnel is up: run \`$(basename "$0") up\` first"
    cat <<ENV
export CUSTOMER_CLOUD_ENABLED=true
export CUSTOMER_CLOUD_CONTROL_PLANE_URL=${url}
export CUSTOMER_CLOUD_HERALD_ISSUER=${url}/realms/${REALM}
ENV
}

write_proxy_config() {
    cat >"${STATE_DIR}/default.conf" <<CONF
server {
    listen 80;

    location = /realms/${REALM}/protocol/openid-connect/token {
        proxy_pass http://host.docker.internal:${IDP_PORT};
        proxy_set_header Host localhost:${IDP_PORT};
        add_header X-Autharie-Edge realm-token always;
    }

    location / {
        proxy_pass http://host.docker.internal:${API_PORT};
        proxy_set_header Host localhost:${API_PORT};
    }
}
CONF
}

# Copied in rather than bind-mounted: a file rewritten in place is read stale
# through a bind mount on a Mac, and nginx then refuses a truncated config.
start_proxy() {
    docker rm -f "${EDGE_NAME}" >/dev/null 2>&1 || true
    docker create --name "${EDGE_NAME}" \
        -p "127.0.0.1:${EDGE_PORT}:80" \
        "${EDGE_IMAGE}" >/dev/null
    docker cp "${STATE_DIR}/default.conf" "${EDGE_NAME}:/etc/nginx/conf.d/default.conf"
    docker start "${EDGE_NAME}" >/dev/null
}

start_tunnel() {
    : >"${LOG_FILE}"
    case "${TUNNEL}" in
    cloudflared)
        # An empty config on purpose: without it cloudflared reads
        # ~/.cloudflared/config.yml and joins whatever named tunnel that file
        # describes, instead of opening a quick tunnel of its own.
        : >"${STATE_DIR}/cloudflared.yml"
        nohup cloudflared tunnel --config "${STATE_DIR}/cloudflared.yml" \
            --no-autoupdate --url "http://localhost:${EDGE_PORT}" \
            >"${LOG_FILE}" 2>&1 &
        ;;
    ngrok)
        nohup ngrok http "${EDGE_PORT}" --log=stdout >"${LOG_FILE}" 2>&1 &
        ;;
    *)
        die "EXPOSE_WITH must be cloudflared or ngrok, not ${TUNNEL}"
        ;;
    esac
    echo $! >"${PID_FILE}"
}

discover_url() {
    local url=""
    for _ in $(seq 1 40); do
        case "${TUNNEL}" in
        cloudflared)
            url="$(grep -Eo 'https://[a-z0-9-]+\.trycloudflare\.com' "${LOG_FILE}" | head -1 || true)"
            ;;
        ngrok)
            url="$(curl -fsS http://127.0.0.1:4040/api/tunnels 2>/dev/null |
                jq -r '.tunnels[0].public_url // empty' 2>/dev/null || true)"
            ;;
        esac
        [ -n "${url}" ] && break
        tunnel_alive || die "the tunnel stopped; see ${LOG_FILE}"
        sleep 1
    done
    [ -n "${url}" ] || die "the tunnel gave no URL after 40 seconds; see ${LOG_FILE}"
    printf '%s' "${url}" >"${URL_FILE}"
}

probe() {
    curl -sS -D - -o /dev/null --max-time 5 -X POST \
        -d grant_type=client_credentials "$1" 2>/dev/null || true
}

code_of() {
    printf '%s' "$1" | awk 'NR == 1 { print $2 }'
}

# No answer at all, or a 5xx: the tunnel or the origin is not there yet.
answered() {
    [ -n "$1" ] && [ "$1" -lt 500 ]
}

check() {
    local url="$1" token master master_code="" token_code=""
    # The name of a quick tunnel takes a moment to resolve from outside, and
    # its edge connection a moment more.
    for _ in $(seq 1 40); do
        master="$(probe "${url}/realms/master/protocol/openid-connect/token")"
        master_code="$(code_of "${master}")"
        answered "${master_code}" && break
        sleep 2
    done
    token="$(probe "${url}/realms/${REALM}/protocol/openid-connect/token")"
    token_code="$(code_of "${token}")"

    echo "   token route of the ${REALM} realm: HTTP ${token_code:-none}"
    echo "   token route of the master realm:   HTTP ${master_code:-none} (not the realm: the control plane answers it)"
    answered "${master_code}" || die "nothing answers through the tunnel after 80 seconds; see ${LOG_FILE}"
    if printf '%s' "${master}" | grep -qi '^x-autharie-edge:'; then
        die "the master realm is reachable from outside: stopping"
    fi
    if ! printf '%s' "${token}" | grep -qi '^x-autharie-edge: realm-token'; then
        die "the token route of the ${REALM} realm is not reached through the tunnel"
    fi
}

cmd_up() {
    require docker
    require curl
    case "${TUNNEL}" in
    cloudflared) require cloudflared ;;
    ngrok)
        require ngrok
        require jq
        ;;
    esac
    mkdir -p "${STATE_DIR}"
    cmd_down quiet

    echo "▸ proxy on 127.0.0.1:${EDGE_PORT}"
    write_proxy_config
    start_proxy

    echo "▸ ${TUNNEL} tunnel"
    start_tunnel
    discover_url

    local url
    url="$(public_url)"
    echo "▸ checking ${url}"
    check "${url}"

    cat <<DONE

✅ exposed at ${url}

Start the control plane with it:

$(print_env | sed 's/^/  /')

  make demo

Stop it with: scripts/expose.sh down
DONE
}

cmd_down() {
    if tunnel_alive; then
        kill "$(cat "${PID_FILE}")" 2>/dev/null || true
    fi
    rm -f "${PID_FILE}" "${URL_FILE}"
    docker rm -f "${EDGE_NAME}" >/dev/null 2>&1 || true
    [ "${1:-}" = "quiet" ] || echo "✅ tunnel and proxy stopped"
}

cmd_status() {
    local url
    url="$(public_url || true)"
    if tunnel_alive && [ -n "${url}" ]; then
        echo "up: ${url}"
    else
        echo "down"
    fi
}

case "${1:-}" in
up) cmd_up ;;
down) cmd_down ;;
env) print_env ;;
status) cmd_status ;;
*)
    usage
    exit 1
    ;;
esac
