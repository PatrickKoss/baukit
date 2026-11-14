{{- define "baukit-platform-observability.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{- define "baukit-platform-observability.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := include "baukit-platform-observability.name" . -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end }}

{{- define "baukit-platform-observability.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | quote }}
app.kubernetes.io/name: {{ include "baukit-platform-observability.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/part-of: baukit-platform
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/* Component of a pack file: the first dot-separated part of its base name. Fails when values do not list it. */}}
{{- define "baukit-platform-observability.component" -}}
{{- $component := index (splitList "." (base .path)) 0 -}}
{{- if not (hasKey .root.Values.components $component) -}}
{{- fail (printf "%s belongs to component %q, which is missing from .Values.components" .path $component) -}}
{{- end -}}
{{- $component -}}
{{- end }}

{{/* Resource name for a pack file: <fullname>-<file name without extensions>. */}}
{{- define "baukit-platform-observability.fileName" -}}
{{- $stem := base .path | trimSuffix ".json" | trimSuffix ".yml" | trimSuffix ".yaml" | trimSuffix ".rules" -}}
{{- printf "%s-%s" (include "baukit-platform-observability.fullname" .root) ($stem | replace "." "-") | trunc 63 | trimSuffix "-" -}}
{{- end }}
