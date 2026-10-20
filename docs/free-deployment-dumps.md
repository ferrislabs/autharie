# Dumps of a free deployment

A free deployment has no database cluster of its own. Its database is one
database on the cluster the data plane shares between free deployments, so there
is no CloudNativePG base backup to take for it. What it has instead is a dump:
a `pg_dump` of that one database, written to the object store once a day.

## What is dumped

The deployment's own database and nothing else. Not the shared cluster, not
another tenant's database, not the roles. The dump is taken with the tenant's
own role, the same one FerrisKey connects with, so it cannot read anything that
role cannot.

It is `pg_dump -Fc` (custom format) with `--no-owner --no-acl`. Custom format is
what lets `pg_restore` restore a single table or skip the data; leaving owners
and grants out means a restore into the same database does not fail on a role
that is not there.

## Where it goes and how long it stays

```
<bucket>/dumps/<deployment-id>/<UTC timestamp>.dump
```

The bucket is the one in the deployment's backup destination
(`s3://<bucket>/...`). The timestamp looks like `20261020T023117Z`, so names
sort in the order they were taken.

After every upload the job deletes everything under that prefix except the seven
newest. A week of dailies. A failed dump deletes nothing: the upload runs first,
so the retention step never removes the only good copy to make room for a bad
one.

Every dump asks the object store for server side encryption, the same as every
other object this platform writes (`AES256` unless the destination asks for
`aws:kms`). What that does and does not protect against is in
[backup-encryption.md](backup-encryption.md). It applies here too: the store
holds the key.

## When it runs

A CronJob named `deployment-<id>-dump` in the deployment's namespace. The time
is fixed per deployment, taken from its id, somewhere between 01:00 and 05:00
UTC. It is spread on purpose so that free deployments do not all open a
connection to the shared cluster in the same minute. It does not change between
reconciles.

The job never overlaps itself, keeps its last successful run and its last three
failed ones, and gives up after an hour. Look at it with:

```bash
kubectl -n <tenant-namespace> get cronjob,job
kubectl -n <tenant-namespace> logs job/<job-name> -c upload
kubectl -n <tenant-namespace> logs job/<job-name> -c pg-dump
```

The dump runs the same Postgres image as the shared cluster, read from the
cluster's status, because `pg_dump` refuses a server newer than itself. Until
the cluster reports its image the CronJob is not created, and the operator
retries every 30 seconds.

An instance with no backup destination gets no CronJob, and loses the one it had
if the destination is removed.

## When the deployment is deleted

The CronJob is in the tenant namespace, owned by the instance, and goes with
it. **The dumps stay in the object store.** They are the recovery, and a
recovery that is deleted together with the thing it recovers is not one. The
seven that were there stay seven: nothing runs any more to prune them, so
removing them is a deliberate act of whoever operates the store.

## Restoring by hand

There is no console restore yet. This is the manual way, to a database on the
shared cluster. Everything in angle brackets is a placeholder.

1. Find the dump you want.

   ```bash
   aws s3 ls s3://<bucket>/dumps/<deployment-id>/ \
     --endpoint-url <object-store-endpoint>
   ```

2. Download it somewhere that can reach the shared cluster, or run the next
   steps from a pod in the data plane.

   ```bash
   aws s3 cp s3://<bucket>/dumps/<deployment-id>/<timestamp>.dump ./restore.dump \
     --endpoint-url <object-store-endpoint>
   ```

3. Stop the tenant's FerrisKey from writing while you restore, so nothing
   changes under you.

   ```bash
   kubectl -n <tenant-namespace> scale deployment <api-deployment> --replicas=0
   ```

   The operator puts it back on its next reconcile. Do the restore before that
   or suspend the instance first.

4. Restore into the tenant's own database, with the tenant's own role. The
   connection values are in the tenant's database Secret
   (`host`, `port`, `database`, `user`, `password`).

   ```bash
   PGPASSWORD='<password>' pg_restore \
     --host <host> --port <port> \
     --username <user> --dbname <database> \
     --clean --if-exists --no-owner --no-acl \
     restore.dump
   ```

   `--clean --if-exists` drops what is there and replaces it. To restore into an
   empty database instead, create it first and leave those two flags off.

5. Scale the API back up, or let the operator do it.

   ```bash
   kubectl -n <tenant-namespace> scale deployment <api-deployment> --replicas=1
   ```

6. Check that an administrator can sign in at the deployment's address.

Use a `pg_restore` of the same Postgres major as the shared cluster or newer (the dump's own `pg_dump` image is the one to use).
An older one refuses a dump taken by a newer `pg_dump`.

## Locally

With the local stack, the same job runs against RustFS. The object store needs
`RUSTFS_SSE_S3_MASTER_KEY` set before it accepts an encrypted write, as
described in [backup-encryption.md](backup-encryption.md). To see a dump
without waiting for the night, make a job from the CronJob:

```bash
kubectl --context=<local-context> -n <tenant-namespace> \
  create job --from=cronjob/deployment-<id>-dump manual-dump
```
