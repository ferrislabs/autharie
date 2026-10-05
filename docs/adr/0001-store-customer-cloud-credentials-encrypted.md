# 0001. Store customer cloud credentials encrypted in the control plane

Status: proposed
Date: 2026-10-04

## Context
Issue #29 keeps the Scaleway credential in control-plane configuration, out of the domain, payloads and logs. That works for one credential owned by the platform. A customer cloud provider needs one credential per customer, registered at runtime, used at creation and again at teardown, possibly weeks later. Configuration cannot hold that. The control plane already wraps backup keys with an OpenBao transit key provider.

## Decision
Store each customer credential as ciphertext in PostgreSQL, wrapped with the transit key provider. The domain handles an id and a secret type that does not implement `Debug` or `Serialize`. The secret is decrypted only inside the provisioner call and is dropped after it. The API never returns it.

## Alternatives rejected
- Kubernetes secret per customer: spreads secrets across the cluster, no audit of use, and the control plane would still need the value to provision.
- Ask the customer for the credential at each operation: teardown cannot run unattended, so a failed deployment leaves a cluster billing on their account.
- Short-lived credentials from the provider (workload identity federation): the best long-term shape, not offered consistently across providers. Revisit per provider.

## Consequences
- Gains: unattended teardown, one place to audit and rotate, same key infrastructure as backups.
- Costs: the control plane now holds secrets that can create infrastructure in customer accounts. A control plane compromise reaches them.
- What becomes harder: key rotation must re-wrap stored credentials. The permission check at registration (no more than required) is the main mitigation and is part of the spec.
