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

{{/* Deployment selector: process labels plus the baukit.dev/workload label Flagger rewrites to <name>-primary. */}}
{{- define "baukit-app.workloadSelectorLabels" -}}
{{ include "baukit-app.processSelectorLabels" . }}
baukit.dev/workload: {{ printf "%s-%s" (include "baukit-app.fullname" .root) .process }}
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
{{- include "baukit-app.backingEnv" . }}
{{- with $root.Values.config.otlpEndpoint }}
- name: OTEL_EXPORTER_OTLP_ENDPOINT
  value: {{ . | quote }}
{{- end }}
{{- $overrides := deepCopy $root.Values.config.overrides -}}
{{- if and (include "baukit-app.redisClient" .) (include "baukit-app.redisMode" $root) }}
{{- if not (hasKey $overrides "RATE_LIMIT__REDIS_KEY_NAMESPACE") }}
{{- $_ := set $overrides "RATE_LIMIT__REDIS_KEY_NAMESPACE" (include "baukit-app.redisKeyNamespace" $root) -}}
{{- end }}
{{- end }}
{{- if eq $process "api" }}
{{- if $root.Values.mcp.enabled }}
{{- if empty $root.Values.mcp.allowedHosts }}
{{- fail "mcp.allowedHosts must not be empty when MCP is enabled" }}
{{- end }}
{{- $_ := set $overrides "MCP__ENABLED" "true" -}}
{{- $_ := set $overrides "MCP__RESOURCE_URL" (required "mcp.resourceUrl is required" $root.Values.mcp.resourceUrl) -}}
{{- $_ := set $overrides "MCP__ISSUER" (required "mcp.issuer is required" $root.Values.mcp.issuer) -}}
{{- with $root.Values.mcp.jwksUri }}
{{- $_ := set $overrides "MCP__JWKS_URI" . -}}
{{- end }}
{{- $_ := set $overrides "MCP__ALLOWED_HOSTS" (toJson $root.Values.mcp.allowedHosts) -}}
{{- $_ := set $overrides "MCP__ALLOWED_ORIGINS" (toJson $root.Values.mcp.allowedOrigins) -}}
{{- $_ := set $overrides "MCP__MAX_REQUEST_BODY_BYTES" (toString $root.Values.mcp.maxRequestBodyBytes) -}}
{{- $_ := set $overrides "MCP__REQUESTS_PER_MINUTE" (toString $root.Values.mcp.requestsPerMinute) -}}
{{- end }}
{{- if $root.Values.browserSession.enabled }}
{{- $session := $root.Values.browserSession -}}
{{- if empty $session.allowedOrigins }}
{{- fail "browserSession.allowedOrigins must list the web origins when browser sessions are enabled" }}
{{- end }}
{{- if empty $session.redirectUris }}
{{- fail "browserSession.redirectUris must not be empty when browser sessions are enabled" }}
{{- end }}
{{- $_ := set $overrides "AUTH__BROWSER_SESSION__ENABLED" "true" -}}
{{- $_ := set $overrides "HTTP__CORS_ALLOWED_ORIGINS" (toJson $session.allowedOrigins) -}}
{{- $_ := set $overrides "AUTH__BROWSER_SESSION__REDIRECT_URIS" (toJson $session.redirectUris) -}}
{{- $_ := set $overrides "AUTH__BROWSER_SESSION__PUBLIC_API_ORIGIN" (required "browserSession.publicApiOrigin is required" $session.publicApiOrigin) -}}
{{- with $session.postLogoutRedirectUri }}
{{- $_ := set $overrides "AUTH__BROWSER_SESSION__POST_LOGOUT_REDIRECT_URI" . -}}
{{- end }}
- name: {{ printf "%s__AUTH__BROWSER_SESSION__CREDENTIAL_KEYS" $prefix }}
  valueFrom:
    secretKeyRef:
      name: {{ required "browserSession.credentialKeysSecretRef.name is required" $session.credentialKeysSecretRef.name | quote }}
      key: {{ required "browserSession.credentialKeysSecretRef.key is required" $session.credentialKeysSecretRef.key | quote }}
{{- end }}
{{- with $root.Values.platform.receiverSecretRef.name }}
- name: {{ printf "%s__PLATFORM__RECEIVER_SECRET" $prefix }}
  valueFrom:
    secretKeyRef:
      name: {{ . | quote }}
      key: {{ required "platform.receiverSecretRef.key is required" $root.Values.platform.receiverSecretRef.key | quote }}
{{- with $root.Values.platform.receiverSecretRef.previousKey }}
- name: {{ printf "%s__PLATFORM__PREVIOUS_RECEIVER_SECRET" $prefix }}
  valueFrom:
    secretKeyRef:
      name: {{ $root.Values.platform.receiverSecretRef.name | quote }}
      key: {{ . | quote }}
{{- end }}
{{- end }}
{{- $_ := set $overrides "HTTP__PORT" (toString $root.Values.api.ports.http) -}}
{{- $_ := set $overrides "OPS__PORT" (toString $root.Values.api.ports.ops) -}}
{{- $_ := set $overrides "SHUTDOWN__DRAIN_TIMEOUT" (include "baukit-app.apiDrainSeconds" $root) -}}
{{- else if and (eq $process "worker") $root.Values.worker.ops.enabled }}
{{- $_ := set $overrides "OPS__PORT" (toString $root.Values.worker.ops.port) -}}
{{- $_ := set $overrides "SHUTDOWN__DRAIN_TIMEOUT" (include "baukit-app.workerDrainSeconds" $root) -}}
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

{{/* Render a pinned image reference from (dict "image" <values> "name" <values path>). */}}
{{- define "baukit-app.image" -}}
{{- $image := .image -}}
{{- $repository := required (printf "%s.image.repository is required" .name) $image.repository -}}
{{- $tag := toString (default "" $image.tag) -}}
{{- $digest := toString (default "" $image.digest) -}}
{{- if eq $tag "latest" -}}
{{- fail (printf "%s.image.tag must not be latest; pin a release tag or set %s.image.digest" .name .name) -}}
{{- end -}}
{{- if $digest -}}
{{- if not (regexMatch "^sha256:[a-f0-9]{64}$" $digest) -}}
{{- fail (printf "%s.image.digest must look like sha256:<64 hex characters>" .name) -}}
{{- end -}}
{{- if $tag -}}
{{- printf "%s:%s@%s" $repository $tag $digest -}}
{{- else -}}
{{- printf "%s@%s" $repository $digest -}}
{{- end -}}
{{- else if $tag -}}
{{- printf "%s:%s" $repository $tag -}}
{{- else -}}
{{- fail (printf "%s.image.tag or %s.image.digest is required" .name .name) -}}
{{- end -}}
{{- end }}

{{/* ServiceAccount used by every pod in the release. */}}
{{- define "baukit-app.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
{{- default (include "baukit-app.fullname" .) .Values.serviceAccount.name -}}
{{- else -}}
{{- required "serviceAccount.name is required when serviceAccount.create is false" .Values.serviceAccount.name -}}
{{- end -}}
{{- end }}

{{/* Pod spec fields shared by every process: (dict "root" $ "process" <name>). */}}
{{- define "baukit-app.podIdentity" -}}
serviceAccountName: {{ include "baukit-app.serviceAccountName" .root }}
automountServiceAccountToken: {{ .root.Values.serviceAccount.automountToken }}
{{- end }}

{{/* Soft node spread for a serving process: (dict "root" $ "process" <name>). */}}
{{- define "baukit-app.topologySpread" -}}
{{- $spread := .root.Values.topologySpread -}}
topologySpreadConstraints:
  - maxSkew: {{ $spread.maxSkew }}
    topologyKey: {{ $spread.topologyKey }}
    whenUnsatisfiable: {{ $spread.whenUnsatisfiable }}
    labelSelector:
      matchLabels:
        {{- include "baukit-app.processSelectorLabels" . | nindent 8 }}
{{- end }}

{{/* Writable /tmp for read-only root filesystems. */}}
{{- define "baukit-app.tmpVolumeMount" -}}
- name: tmp
  mountPath: /tmp
{{- end }}

{{- define "baukit-app.tmpVolume" -}}
- name: tmp
  emptyDir: {}
{{- end }}

{{/* The chart owns the tmp volume and the /tmp mount; values that add either would collide. */}}
{{- define "baukit-app.assertNoTmpOverride" -}}
{{- range .volumes }}
{{- if eq (toString .name) "tmp" }}
{{- fail (printf "%s.volumes must not define a volume named tmp; the chart already mounts one at /tmp" $.process) }}
{{- end }}
{{- end }}
{{- range .volumeMounts }}
{{- if or (eq (toString .name) "tmp") (eq (trimSuffix "/" (toString .mountPath)) "/tmp") }}
{{- fail (printf "%s.volumeMounts must not mount tmp or /tmp; the chart already mounts an emptyDir at /tmp" $.process) }}
{{- end }}
{{- end }}
{{- end }}

{{/* API drain timeout: grace minus the preStop sleep minus the flush and exit slack. */}}
{{- define "baukit-app.apiDrainSeconds" -}}
{{- $grace := int .Values.api.terminationGracePeriodSeconds -}}
{{- $sleep := int .Values.api.preStopSleepSeconds -}}
{{- $slack := 5 -}}
{{- $drain := sub (sub $grace $sleep) $slack -}}
{{- if le $drain 0 -}}
{{- fail (printf "api.terminationGracePeriodSeconds (%d) must exceed api.preStopSleepSeconds (%d) plus %d s of shutdown slack" $grace $sleep $slack) -}}
{{- end -}}
{{- $drain -}}
{{- end }}

{{/* Worker drain timeout: grace minus the time to release leases and flush telemetry. */}}
{{- define "baukit-app.workerDrainSeconds" -}}
{{- $grace := int .Values.worker.terminationGracePeriodSeconds -}}
{{- $reserve := 15 -}}
{{- $drain := sub $grace $reserve -}}
{{- if le $drain 0 -}}
{{- fail (printf "worker.terminationGracePeriodSeconds (%d) must exceed the %d s shutdown reserve" $grace $reserve) -}}
{{- end -}}
{{- $drain -}}
{{- end }}

{{/* Web preStop sleep, checked against the grace period. */}}
{{- define "baukit-app.webPreStopSeconds" -}}
{{- $grace := int .Values.web.terminationGracePeriodSeconds -}}
{{- $sleep := int .Values.web.preStopSleepSeconds -}}
{{- if ge $sleep $grace -}}
{{- fail (printf "web.preStopSleepSeconds (%d) must be lower than web.terminationGracePeriodSeconds (%d)" $sleep $grace) -}}
{{- end -}}
{{- $sleep -}}
{{- end }}

{{/* Traefik reference to the API retry Middleware. */}}
{{- define "baukit-app.retryMiddlewareName" -}}
{{- printf "%s-retry" (include "baukit-app.fullname" .) -}}
{{- end }}

{{/* Render an IntOrString value, quoting percentages. */}}
{{- define "baukit-app.intOrPercent" -}}
{{- if kindIs "string" . }}{{ . | quote }}{{ else }}{{ . }}{{ end -}}
{{- end }}
