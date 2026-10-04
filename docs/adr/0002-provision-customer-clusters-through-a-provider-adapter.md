# 0002. Provision customer clusters through a native provider adapter

Status: proposed
Date: 2026-10-04

## Context
`ClusterProvisioner` has `provision` and `deprovision`, with a local adapter that creates nothing. The Scaleway adapter (#29) must create a Kapsule cluster with a node pool, wait for it, hand a kubeconfig to bootstrap, and delete everything on teardown, including after a crash halfway through creation. Customer clusters add resizing and an inventory of what was created.

## Decision
Implement the adapter against the provider API directly in Rust, one resource at a time, recording each created resource id before moving on. Teardown deletes from that record, not from what the adapter believes it did.

## Alternatives rejected
- OpenTofu or Terraform in a job: gives a reliable `destroy` and state, but adds a binary, a state backend holding customer account details, and a second language for failures. It is the better choice if resource count grows past a cluster and a node pool.
- Crossplane: an operator and its provider packages for two resources.

## Consequences
- Gains: no extra runtime, errors map to `ProvisionError` variants directly, the inventory is a first-class domain record.
- Costs: idempotent create, resume and delete are ours to write and test.
- What becomes harder: a second provider reimplements the sequence. Reopen this ADR when the second provider starts, or when the resources per cluster exceed a cluster, a pool and a private network.
