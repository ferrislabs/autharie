# Deploying Autharie to production

Autharie is the Autharie control plane, deployed to the `ferriskey-vps` cluster
(context `ferriskey-vps`) via ArgoCD. What's in this repo —
`charts/autharie-control-plane/`, `deploy/argocd/`, `deploy/autharie/`,
`deploy/openbao/` — is everything ArgoCD can sync on its own. This document
is the rest: the one-time, imperative steps that stand up what those
manifests only *reference* (`existingSecret: ...`).

No secret is ever committed to this repo — it is public. Every `Secret`
below is created directly on the cluster with `kubectl`.

## 1. Apply the ArgoCD project and Applications

```bash
kubectl --context ferriskey-vps apply -f deploy/argocd/appproject-autharie.yaml
kubectl --context ferriskey-vps apply -f deploy/argocd/application-openbao.yaml
kubectl --context ferriskey-vps apply -f deploy/argocd/application-autharie-ferriskey-db.yaml
kubectl --context ferriskey-vps apply -f deploy/argocd/application-autharie-ferriskey.yaml
kubectl --context ferriskey-vps apply -f deploy/argocd/application-autharie.yaml
```

`autharie` and `autharie-ferriskey` will fail health checks at first —
their `existingSecret`s do not exist yet. That's expected until step 5.

## 2. OpenBao: init, unseal, transit engine

Wait for the pod:

```bash
kubectl --context ferriskey-vps -n openbao wait --for=condition=Ready pod/openbao-0 --timeout=120s
```

Initialise it. **This prints the unseal keys and the root token once.** Save
them somewhere you control (a password manager) — never in this repo, never
in a cluster Secret readable by anything but you.

```bash
kubectl --context ferriskey-vps -n openbao exec -it openbao-0 -- bao operator init
```

Unseal with three of the five keys just printed:

```bash
kubectl --context ferriskey-vps -n openbao exec -it openbao-0 -- bao operator unseal   # x3
```

Log in with the root token and configure the transit engine autharie wraps
backup keys with:

```bash
kubectl --context ferriskey-vps -n openbao exec -it openbao-0 -- sh -c '
  bao login -
  bao secrets enable -path=transit transit
  bao policy write autharie-backups - <<EOF
path "transit/encrypt/autharie-backups" { capabilities = ["update"] }
path "transit/decrypt/autharie-backups" { capabilities = ["update"] }
path "transit/keys/autharie-backups"    { capabilities = ["read", "create", "update"] }
EOF
  bao token create -policy=autharie-backups -period=768h -orphan
'
```

Create the Secret the chart references (`keyManager.existingSecret:
autharie-key-manager`) from the token the last command printed:

```bash
kubectl --context ferriskey-vps create namespace autharie --dry-run=client -o yaml | kubectl --context ferriskey-vps apply -f -
kubectl --context ferriskey-vps -n autharie create secret generic autharie-key-manager \
  --from-literal=token=<the token>
```

## 3. Ferriskey: realm, console client, admin credentials

Wait for `autharie-ferriskey` to sync and its pods to be ready, then read the
admin password the chart generated:

```bash
kubectl --context ferriskey-vps -n autharie-ferriskey get secret autharie-ferriskey-api-admin \
  -o jsonpath='{.data.password}' | base64 -d
```

Sign in at `https://id.autharie.ferrislabs.fr` as `admin` with that
password, then:

1. Create a realm named **autharie** (matches `realm.name` in
   `charts/autharie-control-plane/values-production.yaml`).
2. Inside it, create a public (SPA) client `autharie-console` with a redirect
   URI of `https://autharie.ferrislabs.fr/*` — this is `oidc.clientId` in the
   same file.
3. In its settings, leave user registration **off**. The beta runs on an
   access list: an account is made when somebody is invited
   (section 7), and an open login page would let
   anyone create an organisation and a free tenant.

Copy the admin credentials into the `autharie` namespace — Ferriskey cannot
hand a Secret across namespaces on its own, and the control plane's
`REALM_ADMIN_*` needs them there:

```bash
kubectl --context ferriskey-vps -n autharie create secret generic autharie-realm-admin \
  --from-literal=username=admin \
  --from-literal=password=<the password from above>
```

## 4. RustFS credentials

Autharie's own object store. Generate real values rather than reusing the
local-dev ones from `docker-compose.yaml`:

```bash
kubectl --context ferriskey-vps -n autharie create secret generic autharie-rustfs \
  --from-literal=access-key=$(openssl rand -hex 12) \
  --from-literal=secret-key=$(openssl rand -hex 24) \
  --from-literal=sse-master-key=$(openssl rand -base64 32)
```

## 5. Sync and verify

```bash
kubectl --context ferriskey-vps -n argocd get application autharie autharie-ferriskey autharie-ferriskey-db openbao
```

All four should reach `Synced`/`Healthy`. Watch the migration job on a first
sync:

```bash
kubectl --context ferriskey-vps -n autharie logs job/autharie-migrations -c migrate -f
```

Then check both hostnames answer: `https://autharie.ferrislabs.fr` (console)
and `https://id.autharie.ferrislabs.fr` (the dedicated Ferriskey). Signing
into the console should redirect through Ferriskey and back.

## 6. The first operator

Nobody can operate the installation yet — `platform.bootstrapOperator` is
empty on purpose (see the comment in `values-production.yaml`). Sign into
the console once, then find your subject in the control plane's logs (it
logs who it could not authorise, with their subject) or decode the id token
your browser received. Set it:

```bash
# in charts/autharie-control-plane/values-production.yaml
platform:
  bootstrapOperator: "<your subject>"
```

Commit, push to `main`, let ArgoCD's `selfHeal` pick it up. Once you have
confirmed you hold platform rights through the console, remove the value and
push again — it is a way back in, not a standing instruction.

## 7. Beta accounts

The beta is on an access list, so nobody can sign up from the login page.
An account is created when somebody is invited to an organisation from the
console: the control plane creates it in the `autharie` realm and asks
FerrisKey to email the invitee a link to set their password (valid for 30
minutes; "Forgot password" on the sign-in page sends a new one). This needs
the control plane's `REALM_ADMIN_*` settings and a working mail setup:

1. Set the SMTP variables and apply the Terraform in
   `deploy/autharie/terraform`: `smtp_host`, `smtp_port` (587),
   `smtp_username`, `smtp_password` (via `TF_VAR_smtp_password`),
   `smtp_from_email`, `smtp_from_name` and `smtp_encryption`
   (`tls`, `starttls` or `none`). The apply also turns on
   `forgot_password_enabled` for the realm.
2. Once, in FerrisKey's admin UI for realm `autharie`, create a
   `reset_password` email template and select it as the realm's
   reset-password template. The Terraform provider cannot do this.
3. Invite people from the console's Members page. Inviting an address that
   already has an account creates nothing new, and revoking or letting an
   invitation expire never deletes an account.

Without `REALM_ADMIN_*` the control plane logs a warning at startup and an
invitation only returns its link. If the set-password email cannot be
requested the invitation is still stored, and the invitee can use "Forgot
password" instead.

### Fallback: no SMTP

`scripts/invite-user.sh` remains for an installation without SMTP. It makes
one account per person and prints a temporary password once; FerrisKey asks
for a new one at the first sign-in:

```bash
FERRISKEY_URL=https://id.autharie.fr/api \
FERRISKEY_ADMIN_USER=admin \
FERRISKEY_ADMIN_PASSWORD=<the password from section 3> \
  scripts/invite-user.sh someone@example.com
```

Send the password through a different channel than the email address. An
address that already has an account is refused (FerrisKey answers 400).
