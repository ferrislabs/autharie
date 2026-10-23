# 0013. Autonomous agent secrets are sealed to the requesting browser

Status: proposed
Date: 2026-10-23

## Context
A customer creating an autonomous agent (a workflow, a bot) from the console needs its client secret once. FerrisKey only supports secret-based client authentication. The control plane has no credentials to the data plane (ADR 0005): whatever the operator learns travels back through outcomes and the control plane's database.

## Decision
The browser generates an RSA-OAEP-256 key pair for the request and sends only the public key. The operator creates the client, encrypts its secret to that key, and reports the sealed value. The control plane stores the sealed value, hands it out once, and deletes it after delivery or 15 minutes. The browser decrypts and shows it once.

## Alternatives rejected
- Sending the secret in clear through the outcome path: it would sit in Iggy, Herald and the control plane database.
- Encrypting with a control-plane key: the control plane could read every secret.
- private_key_jwt: no secret at all, but FerrisKey does not support it yet.

## Consequences
- Gains: only the data plane and the requesting browser ever see the secret.
- Costs: a lost dialog means rotating the secret; WebCrypto is required in the browser.
- What becomes harder: nothing for later private_key_jwt support, which can replace this path.
