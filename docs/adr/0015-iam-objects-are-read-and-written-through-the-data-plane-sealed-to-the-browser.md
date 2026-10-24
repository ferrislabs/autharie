# 0015. IAM objects are read and written through the data plane, sealed to the browser

Status: proposed
Date: 2026-10-24

## Context
The Identity space (ADR 0014) will list and edit what lives inside a realm: applications, users, organisations, roles. A realm can hold tens of thousands of users, and they are the customer's own end users: personal data that Autharie has no reason to store.

The control plane holds no instance credentials and cannot reach an instance on a private network or in a customer's cloud (ADR 0007). What it knows of FerrisKey comes back from the data plane as bounded snapshots: the agent registry's clients, capped and flagged `clients_truncated`. A snapshot does not fit users: too many, too stale, and it would put them at rest in Autharie. Console features that need an answer now already follow a request and outcome shape: discovery of a customer's MCP server is requested by an action and read back by polling, and an autonomous agent's secret is sealed to the requesting browser (ADR 0013).

FerrisKey supports RFC 8693 token exchange with `act` delegation (ADR 0008).

## Decision
One request at a time, through the data plane, with the answer sealed to the browser that asked.

The console asks the control plane for one operation on one realm (`list_users` with a search term and a page, `get_application`, `update_application`...). The control plane checks the caller's right, writes an audit entry (who, what, which realm, never the result), and records an action `deployment.iam.request` carrying the operation, its parameters and a public key the browser generated for this request. The operator runs it against FerrisKey's admin API and reports the result encrypted to that key. The control plane keeps the ciphertext for a few minutes, hands it out once and deletes it. The browser decrypts it and shows it.

Writes use the same path: one operation per request, keyed by the request id, so a redelivery does not apply twice.

## Alternatives rejected
- Mirror users into the control plane's database from snapshots: customers' end users at rest in Autharie, a size that grows with the customer, and a view that is always late.
- The control plane calls the instance's admin API (or exchanges a token for the acting user): synchronous and simple, but it needs reachability and a credential per instance, which ADR 0007 already refused.
- The browser calls the realm directly: the owner has no account in the realm, and every deployment would have to accept the console's origin.
- Send the result in clear through the outcome path: it would sit in the bus and in the control plane.

## Consequences
- Gains: works for every distribution; Autharie's control plane never reads a customer's users, only ciphertext; no new credential in the control plane; every operation leaves an audit entry of who asked.
- Costs: each click takes the round trip of an action and an outcome, seconds rather than milliseconds; search is submitted, not as-you-type; a result is lost if the tab is closed before it arrives; WebCrypto is required, and a result larger than one RSA block needs hybrid encryption (AES-GCM key sealed with RSA-OAEP).
- What becomes harder: anything that needs Autharie to look at the content (cross-realm search, counts for the overview) has to come from what FerrisKey itself answers, or from small bounded snapshots that hold no personal data.

## Measured
- Latency. Herald polls the control plane every 15 seconds in the chart (`herald.pollIntervalSeconds`), though the binary's own default is 3 and its help text says a sweep is one request. An action waits up to 15 seconds to be claimed, and the outcome up to 15 more to be carried back, so a click would take 15 to 30 seconds. Genesis itself reports a status change within about a second, and the console already polls every 3 seconds.
- Size. The bus is not the limit: the SDK allows messages up to 256 MiB, and the server's own ceiling is not checked. The limit is the Secret or ConfigMap that carries the sealed result, 1 MiB, which leaves about 700 KiB once encoded. A page is capped at 50 objects and a result at 512 KiB; a larger one is refused rather than cut. The size of a user record is an estimate, not a measure.
- Token exchange. FerrisKey does exchange tokens and writes `act`, but the actor token is verified against the same realm (`verify_subject_token(actor_token, realm.id())`). A person who has no account in the realm cannot be the actor, so FerrisKey's own audit cannot name an Autharie user. The operator keeps using its admin account, and Autharie's audit entry is where the real actor is recorded.

## Follow-up decision
Set the chart's Herald poll interval to 3 seconds, the binary's default: the cost is one request per sweep per Herald. That brings a request to about 5 to 10 seconds, still slow for a click. If it feels slow once built, the next step is a second, faster claim loop for IAM requests only, or a held-open claim; neither is built now.
