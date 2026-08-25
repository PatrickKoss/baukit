{{/* Expand the chart name. */}}
{{- define "baukit-app.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* Create a release-scoped name. */}}
{{- define "baukit-app.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := include "baukit-app.name" . -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end }}

{{/* Stable Redis name: products use redis://<release>-redis:6379. */}}
{{- define "baukit-app.redisName" -}}
{{- printf "%s-redis" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* Chart label. */}}
{{- define "baukit-app.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* Normalize a value for use as a Kubernetes label value. */}}
{{- define "baukit-app.labelValue" -}}
{{- regexReplaceAll "[^a-z0-9_.-]" (. | lower) "-" | trimAll "-." | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/* Labels common to every chart resource. */}}
{{- define "baukit-app.labels" -}}
helm.sh/chart: {{ include "baukit-app.chart" . }}
{{ include "baukit-app.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/part-of: {{ include "baukit-app.labelValue" .Values.product }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
baukit.dev/product: {{ include "baukit-app.labelValue" .Values.product }}
{{- end }}

{{/* Labels shared by every process in this release. */}}
{{- define "baukit-app.selectorLabels" -}}
app.kubernetes.io/name: {{ include "baukit-app.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/* Add the process identity required by the telemetry identity convention. */}}
{{- define "baukit-app.processLabels" -}}
{{ include "baukit-app.labels" .root }}
app.kubernetes.io/component: {{ .process }}
baukit.dev/process: {{ .process }}
{{- end }}

{{/* Immutable workload selector labels. */}}
{{- define "baukit-app.processSelectorLabels" -}}
{{ include "baukit-app.selectorLabels" .root }}
baukit.dev/process: {{ .process }}
{{- end }}

{{/* Normalize product/config prefix to ConfigLoader's environment convention. */}}
{{- define "baukit-app.envPrefix" -}}
{{- $prefix := default .Values.product .Values.config.envPrefix -}}
{{- $normalized := regexReplaceAll "[^A-Z0-9_]" ($prefix | upper) "_" | trimAll "_" -}}
{{- required "product or config.envPrefix must produce a non-empty environment prefix" $normalized -}}
{{- end }}

{{/* Render shared environment and process-specific listener/drain overrides. */}}
{{- define "baukit-app.env" -}}
{{- $root := .root -}}
{{- $process := .process -}}
{{- $prefix := include "baukit-app.envPrefix" $root -}}
{{- if not (has $root.Values.deploymentEnvironment (list "local" "testing" "staging" "production")) -}}
{{- fail "deploymentEnvironment must be local, testing, staging, or production" -}}
{{- end -}}
- name: {{ printf "%s_ENVIRONMENT" $prefix }}
  value: {{ $root.Values.deploymentEnvironment | quote }}
{{- with $root.Values.config.otlpEndpoint }}
- name: OTEL_EXPORTER_OTLP_ENDPOINT
  value: {{ . | quote }}
{{- end }}
{{- $overrides := deepCopy $root.Values.config.overrides -}}
{{- if eq $process "api" }}
{{- $_ := set $overrides "HTTP__PORT" (toString $root.Values.api.ports.http) -}}
{{- $_ := set $overrides "OPS__PORT" (toString $root.Values.api.ports.ops) -}}
{{- $_ := set $overrides "SHUTDOWN__DRAIN_TIMEOUT" (toString $root.Values.api.terminationGracePeriodSeconds) -}}
{{- else if and (eq $process "worker") $root.Values.worker.ops.enabled }}
{{- $_ := set $overrides "OPS__PORT" (toString $root.Values.worker.ops.port) -}}
{{- $_ := set $overrides "SHUTDOWN__DRAIN_TIMEOUT" (toString $root.Values.worker.terminationGracePeriodSeconds) -}}
{{- end }}
{{- range $key, $value := $overrides }}
- name: {{ printf "%s__%s" $prefix (regexReplaceAll "[^A-Z0-9_]" ($key | upper) "_") }}
  value: {{ $value | toString | quote }}
{{- end }}
{{- end }}

{{/* Render envFrom references without ever accepting secret values. */}}
{{- define "baukit-app.envFrom" -}}
{{- range .Values.config.existingSecretRefs }}
- secretRef:
    name: {{ required "config.existingSecretRefs[].name is required" .name }}
    {{- if hasKey . "optional" }}
    optional: {{ .optional }}
    {{- end }}
{{- end }}
{{- end }}

{{/* Render an image reference. */}}
{{- define "baukit-app.image" -}}
{{- printf "%s:%s" .repository (.tag | default "latest") -}}
{{- end }}
