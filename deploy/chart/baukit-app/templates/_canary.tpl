{{/* Flagger's TraefikService that splits traffic between <fullname>-api-primary and -canary. */}}
{{- define "baukit-app.canaryServiceName" -}}
{{- printf "%s-api" (include "baukit-app.fullname" .) -}}
{{- end }}

{{/* Address of the Prometheus API that the chart's MetricTemplates query. */}}
{{- define "baukit-app.canaryPrometheusAddress" -}}
{{- default (printf "http://tenant-metrics-proxy.%s.svc.cluster.local:8080" .Release.Namespace) .Values.canary.prometheusAddress -}}
{{- end }}

{{/* Literal Flagger template placeholder, which Helm must not expand. */}}
{{- define "baukit-app.flaggerInterval" -}}
{{ "{{" }} interval {{ "}}" }}
{{- end }}

{{/* Label matchers for the API canary series. */}}
{{- define "baukit-app.canarySelector" -}}
product="{{ include "baukit-app.labelValue" .Values.product }}", service="{{ include "baukit-app.labelValue" .Values.product }}-api", environment="{{ .Values.deploymentEnvironment }}", rollout="canary"
{{- end }}

{{/* IngressRoute to the Flagger TraefikService. Traefik's Ingress provider cannot target a TraefikService. */}}
{{- define "baukit-app.canaryIngressRoute" -}}
{{- $root := . -}}
{{- $middlewares := list -}}
{{- with $root.Values.ingress.rateLimitMiddleware -}}
{{- $middlewares = append $middlewares . -}}
{{- end -}}
{{- if $root.Values.ingress.retry.enabled -}}
{{- $middlewares = append $middlewares (include "baukit-app.retryMiddlewareName" $root) -}}
{{- end -}}
{{- if $root.Values.ingress.certManager.clusterIssuer -}}
{{- fail "canary.enabled routes through an IngressRoute, which cert-manager annotations do not cover: clear ingress.certManager.clusterIssuer and issue a Certificate for ingress.tls[0].secretName" -}}
{{- end -}}
{{- $paths := list -}}
{{- if $root.Values.mcp.enabled -}}
{{- range list "/mcp" "/.well-known/oauth-protected-resource" "/.well-known/oauth-protected-resource/mcp" -}}
{{- $paths = append $paths (printf "Path(`%s`)" .) -}}
{{- end -}}
{{- end -}}
apiVersion: traefik.io/v1alpha1
kind: IngressRoute
metadata:
  name: {{ include "baukit-app.fullname" $root }}
  labels:
    {{- include "baukit-app.processLabels" (dict "root" $root "process" "api") | nindent 4 }}
spec:
  entryPoints:
    - websecure
  routes:
    {{- range $root.Values.ingress.hosts }}
    {{- $host := .host }}
    {{- $hostPaths := $paths }}
    {{- range .paths }}
    {{- if eq .pathType "Exact" }}
    {{- $hostPaths = append $hostPaths (printf "Path(`%s`)" .path) }}
    {{- else }}
    {{- $hostPaths = append $hostPaths (printf "PathPrefix(`%s`)" .path) }}
    {{- end }}
    {{- end }}
    {{- range $hostPaths }}
    - kind: Rule
      match: {{ printf "Host(`%s`) && %s" $host . | quote }}
      {{- with $middlewares }}
      middlewares:
        {{- range . }}
        - name: {{ . | quote }}
        {{- end }}
      {{- end }}
      services:
        - kind: TraefikService
          name: {{ include "baukit-app.canaryServiceName" $root }}
    {{- end }}
    {{- end }}
  tls:
    {{- with (first $root.Values.ingress.tls) }}
    secretName: {{ .secretName | quote }}
    {{- else }}
    {}
    {{- end }}
{{- end }}
