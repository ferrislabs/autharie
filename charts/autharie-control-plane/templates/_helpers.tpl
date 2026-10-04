{{- define "autharie-control-plane.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "autharie-control-plane.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name (include "autharie-control-plane.name" .) | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "autharie-control-plane.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/name: {{ include "autharie-control-plane.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/part-of: autharie
{{- end }}

{{- define "autharie-control-plane.image" -}}
{{- $tag := .tag | default .root.Chart.AppVersion -}}
{{- printf "%s/%s/autharie-%s:%s" .root.Values.image.registry .root.Values.image.repository .component $tag -}}
{{- end }}

{{/*
The CNPG cluster's read-write service, reachable in-cluster the moment CNPG
creates it -- before the cluster is actually ready, which is why the
migration job waits on it separately rather than trusting DNS resolving as
proof of readiness.
*/}}
{{- define "autharie-control-plane.postgresqlHost" -}}
{{- if .Values.postgresql.enabled -}}
{{- printf "%s-rw.%s.svc.cluster.local" (include "autharie-control-plane.fullname" .) .Release.Namespace -}}
{{- else -}}
{{ .Values.database.host }}
{{- end -}}
{{- end }}

{{- define "autharie-control-plane.postgresqlSecretName" -}}
{{- if .Values.postgresql.enabled -}}
{{- printf "%s-app" (include "autharie-control-plane.fullname" .) -}}
{{- else -}}
{{ .Values.database.existingSecret }}
{{- end -}}
{{- end }}

{{/*
RustFS's own service, reachable in-cluster. Mirrors the Postgres helper: an
external store wins over the in-chart one, so pointing at a real bucket does
not require disabling anything twice.
*/}}
{{- define "autharie-control-plane.objectStoreEndpoint" -}}
{{- if .Values.rustfs.enabled -}}
{{- printf "http://%s-rustfs.%s.svc.cluster.local:9000" (include "autharie-control-plane.fullname" .) .Release.Namespace -}}
{{- else -}}
{{ .Values.objectStore.endpoint }}
{{- end -}}
{{- end }}

{{/*
Fails the render rather than installing something that cannot work. A
control plane with no database and no way to find one starts and fails every
request, which looks like a bug rather than a missing value.
*/}}
{{- define "autharie-control-plane.validate" -}}
{{- if and (not .Values.postgresql.enabled) (not .Values.database.host) -}}
{{- fail "database.host is required when postgresql.enabled is false" -}}
{{- end -}}
{{- if and (not .Values.postgresql.enabled) (not .Values.database.existingSecret) -}}
{{- fail "database.existingSecret is required when postgresql.enabled is false" -}}
{{- end -}}
{{- if and (not .Values.rustfs.enabled) (not .Values.objectStore.endpoint) -}}
{{- fail "objectStore.endpoint is required when rustfs.enabled is false" -}}
{{- end -}}
{{- end }}
