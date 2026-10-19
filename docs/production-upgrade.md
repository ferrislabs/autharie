# Upgrading production and opening free tenants

Production was installed from `NathaelB/aether` before the project moved to
`ferrislabs/autharie`. The ArgoCD Applications still point there, at a
repository frozen on 2026-10-03, so nothing merged since reaches production.
Part A moves production onto this repository. Part B opens free tenants to
the people invited to the beta.

Every step here is run by a person. Commands that change the cluster are
preceded by the read-only command that shows what they will change.

Facts this relies on, read on 2026-10-20:

- The four Applications (`autharie`, `autharie-ferriskey`,
  `autharie-ferriskey-db`, `openbao`) and the `autharie` AppProject source
  `https://github.com/NathaelB/aether`.
- `charts/autharie-control-plane` renders the same 16 resources, with the same
  names, as the old chart. Nothing is pruned by the switch. The CNPG `Cluster`
  and the RustFS PVC render identically. The images move from
  `ghcr.io/nathaelb/aether-*` to `ghcr.io/ferrislabs/autharie-*` (public), and
  `AETHER_BOOTSTRAP_OPERATOR` becomes `AUTHARIE_BOOTSTRAP_OPERATOR`.
- The secrets the chart needs already exist in `autharie`: `autharie-app`,
  `autharie-key-manager`, `autharie-realm-admin`, `autharie-rustfs`.
- The `autharie` database has no CNPG backup configured.
- The OpenBao policy only covers the transit key `autharie-backups`.

## Part A: move production to ferrislabs/autharie

### A1. Back up the database

About 25 migrations run on the first sync. Take a dump first:

```bash
kubectl --context ferriskey-vps -n autharie exec autharie-1 -- \
  pg_dump -U postgres -Fc autharie > autharie-$(date +%Y%m%d-%H%M).dump
ls -lh autharie-*.dump
```

### A2. Widen the OpenBao policy

The control plane wraps keys under three transit keys, not one:
`autharie-backups` (archives), `cloud-credentials` (customer cloud accounts)
and `tenant-administrators` (the first administrator of a free realm). With
only the first, issuing a free tenant's administrator fails. Log in to OpenBao
with the root token, then:

```bash
bao policy read autharie-backups          # what is there today
bao policy write autharie-backups - <<EOF
path "transit/encrypt/autharie-backups"       { capabilities = ["update"] }
path "transit/decrypt/autharie-backups"       { capabilities = ["update"] }
path "transit/keys/autharie-backups"          { capabilities = ["read", "create", "update"] }
path "transit/encrypt/cloud-credentials"      { capabilities = ["update"] }
path "transit/decrypt/cloud-credentials"      { capabilities = ["update"] }
path "transit/keys/cloud-credentials"         { capabilities = ["read", "create", "update"] }
path "transit/encrypt/tenant-administrators"  { capabilities = ["update"] }
path "transit/decrypt/tenant-administrators"  { capabilities = ["update"] }
path "transit/keys/tenant-administrators"     { capabilities = ["read", "create", "update"] }
EOF
```

The token in `autharie-key-manager` keeps the policy name, so it does not
change.

### A3. Merge the chart change first

Merge the PR that makes the chart pull `latest` every time (`imagePullPolicy:
Always` for a `latest` tag). Without it, a node that already holds a `latest`
image keeps running it after every later merge.

### A4. Point ArgoCD at this repository

The project first, so the Applications are allowed to use the new URL:

```bash
cd deploy/argocd
kubectl --context ferriskey-vps diff -f appproject-autharie.yaml
kubectl --context ferriskey-vps apply -f appproject-autharie.yaml
```

The project now allows only `ferrislabs/autharie`. Then each Application, the
database before what uses it:

```bash
for app in application-autharie-ferriskey-db application-autharie-ferriskey \
           application-openbao application-autharie; do
  kubectl --context ferriskey-vps diff -f $app.yaml
done
```

Each diff should show only `repoURL` (and the chart `path` for `autharie`).
Then:

```bash
for app in application-autharie-ferriskey-db application-autharie-ferriskey \
           application-openbao application-autharie; do
  kubectl --context ferriskey-vps apply -f $app.yaml
done
```

### A5. Watch the sync

```bash
kubectl --context ferriskey-vps -n argocd get applications.argoproj.io \
  -o custom-columns=NAME:.metadata.name,SYNC:.status.sync.status,HEALTH:.status.health.status,REV:.status.sync.revision
kubectl --context ferriskey-vps -n autharie get pods -w
kubectl --context ferriskey-vps -n autharie logs job/autharie-migrations -f
```

Expected: the migration Job (a Sync hook, recreated every sync) succeeds, then
`autharie` and `autharie-console` roll onto `ghcr.io/ferrislabs/autharie-*`.
`https://app.autharie.fr` answers and signing in works.

### Rollback

Re-apply the previous Application specs (saved first with
`kubectl --context ferriskey-vps -n argocd get applications.argoproj.io <name> -o yaml`).
The database has moved forward by then: the old code may not start against
the new schema, so restoring the dump from A1 is part of the rollback.

## Part B: free tenants for invited people

A free tenant is a realm on a shared FerrisKey instance, a cell, on a data
plane. Steps B1 to B3 can run in any order; B4 and later need them.

### B1. A FerrisKey release that serves a realm on its own hostname

Tenant login needs ferriskey/ferriskey#1711 (`WEBAPP_URL_FROM_REQUEST`).
Release FerrisKey with it, then raise `CELL_MIN_FERRISKEY` in
`libs/autharie-domain/src/cells/service.rs` to that version, and publish the
release in the catalogue as an operator:

```bash
curl -X POST https://app.autharie.fr/api/releases/operator/ferriskey \
  -H "Authorization: Bearer $OPERATOR_TOKEN" -H 'Content-Type: application/json' \
  -d '{"version":"<version>","risk":"none"}'
curl -X PUT https://app.autharie.fr/api/releases/operator/ferriskey/<version>/status \
  -H "Authorization: Bearer $OPERATOR_TOKEN" -H 'Content-Type: application/json' \
  -d '{"status":"available"}'
```

### B2. DNS zone and wildcard certificate for the control plane

A free deployment gets `{realm}.autharie.fr`, so the control plane needs the
zone and the OVH credentials, and the wildcard certificate it sends to shared
data planes.

The OVH credentials, as a secret (values never in git):

```bash
kubectl --context ferriskey-vps -n autharie create secret generic autharie-ovh \
  --from-literal=application-key=... \
  --from-literal=application-secret=... \
  --from-literal=consumer-key=...
```

The wildcard certificate already exists as `default/autharie-fr-tls`, but the
chart grants read access in the namespace of the certificate, and the
`autharie` project only deploys to `autharie` and `autharie-ferriskey`. So
issue the same wildcard in `autharie`:

```bash
kubectl --context ferriskey-vps apply -f deploy/autharie/certificate-wildcard.yaml
kubectl --context ferriskey-vps -n autharie get certificate autharie-wildcard -w
```

Then, in `charts/autharie-control-plane/values-production.yaml`, through a PR:

```yaml
ovh:
  zone: autharie.fr
  existingSecret: autharie-ovh

certificate:
  secretName: autharie-wildcard-tls
  secretNamespace: autharie
```

### B3. A shared data plane

Register it from the console (platform, data planes) as shared, in the region
free tenants are placed in, and install the chart with the command the console
prints, adding:

```bash
--set genesis.tenantZone=autharie.fr
```

`genesis.tenantZone` must equal `ovh.zone`: Genesis refuses every realm action
whose hostname is not `{realm}.{zone}`. Herald needs `get` and `list` on
`identityinstances`, which the chart carries since #215. One cell asks for
2600m CPU, 4.7 GiB of memory and 20 GiB of storage on the data plane.

### B4. The organisation that owns the cells

As an operator, create it once:

```bash
curl -X POST https://app.autharie.fr/api/organisations \
  -H "Authorization: Bearer $OPERATOR_TOKEN" -H 'Content-Type: application/json' \
  -d '{"name":"Autharie cells"}'
```

Keep only operators as its members. Put its id in `values-production.yaml`
through a PR:

```yaml
platform:
  cellsOrganisationId: <id>
```

### B5. Open a cell

```bash
curl -X POST https://app.autharie.fr/api/platform/cells \
  -H "Authorization: Bearer $OPERATOR_TOKEN" -H 'Content-Type: application/json' \
  -d '{"region":"<region of the data plane>"}'
curl https://app.autharie.fr/api/platform/cells -H "Authorization: Bearer $OPERATOR_TOKEN"
```

It moves from `provisioning` to `open` once its instance runs two replicas on
two database instances, a few minutes.

### B6. Invite people

Check in the FerrisKey admin of `id.autharie.fr` that user registration is off
in the `autharie` realm, then make one account per person:

```bash
FERRISKEY_URL=https://id.autharie.fr/api \
FERRISKEY_ADMIN_USER=admin \
FERRISKEY_ADMIN_PASSWORD=<from autharie-ferriskey-api-admin> \
  scripts/invite-user.sh someone@example.com
```

Each person signs in, changes the temporary password, creates an organisation
and a FerrisKey deployment on the free offer. On the deployment page they
generate and reveal the administrator of their realm, then administer it at
`https://{realm}.autharie.fr/realms/{realm}/overview`.

### B7. Check one tenant yourself

Create a free deployment from an invited account and follow it to
`successful`, reveal its administrator, sign in on its hostname, change the
temporary password and create a client. This is the path that was run on the
local stack on 2026-10-17.
