#!/bin/bash

set -e

cd "$(dirname "$0")/.."

echo "🚀 Generating Autharie CRDs..."

mkdir -p k8s/crds

# Générer la CRD IdentityInstance depuis la lib
echo "📝 Generating IdentityInstance CRD..."
cargo run --quiet -p autharie-crds --example generate_crd -- identity-instance > k8s/crds/identity-instance.yaml

echo "📝 Generating IdentityInstanceUpgrade CRD..."
cargo run --quiet -p autharie-crds --example generate_crd -- identity-instance-upgrade > k8s/crds/identity-instance-upgrade.yaml

echo "📝 Generating IdentityInstanceBackup CRDs..."
cargo run --quiet -p autharie-crds --example generate_crd -- identity-instance-backup > k8s/crds/identity-instance-backup.yaml

echo "📝 Generating IdentityDataplaneUpgrade CRD..."
cargo run --quiet -p autharie-crds --example generate_crd -- identity-dataplane-upgrade > k8s/crds/identity-dataplane-upgrade.yaml

# The data plane chart carries the same definitions, so a cluster that gets the
# chart gets the resources its Genesis and operator work on. Without them both
# fail on a 404 for every resource the platform asks them to create.
mkdir -p charts/autharie-dataplane/crds
cp k8s/crds/*.yaml charts/autharie-dataplane/crds/

echo "✅ CRDs generated successfully:"
echo "  - k8s/crds/identity-instance.yaml"
echo "  - k8s/crds/identity-instance-upgrade.yaml"
echo "  - k8s/crds/identity-instance-backup.yaml"
echo "  - k8s/crds/identity-dataplane-upgrade.yaml"
echo ""
echo "Copied to charts/autharie-dataplane/crds/ as well."
echo ""
echo "To install in your cluster, run:"
echo "  kubectl apply -f k8s/crds/"
