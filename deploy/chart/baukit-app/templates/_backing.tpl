{{/* Validated backing.postgres.mode; empty means the values wire the database. */}}
{{- define "baukit-app.postgresMode" -}}
{{- $mode := toString .Values.backing.postgres.mode -}}
{{- if not (has $mode (list "" "shared" "dedicated")) -}}
{{- fail "backing.postgres.mode must be empty, shared or dedicated" -}}
{{- end -}}
{{- $mode -}}
{{- end }}

{{/* Validated backing.redis.mode; empty means the values wire Redis. */}}
{{- define "baukit-app.redisMode" -}}
{{- $mode := toString .Values.backing.redis.mode -}}
{{- if not (has $mode (list "" "shared" "dedicated")) -}}
{{- fail "backing.redis.mode must be empty, shared or dedicated" -}}
{{- end -}}
{{- $mode -}}
{{- end }}

{{- define "baukit-app.redisEnabled" -}}
{{- if or .Values.redis.enabled (eq (include "baukit-app.redisMode" .) "dedicated") }}true{{ end -}}
{{- end }}

{{/* Whether a process uses the backing Redis: (dict "root" $ "process" <name>). */}}
{{- define "baukit-app.redisClient" -}}
{{- $worker := .root.Values.worker -}}
{{- if or (eq .process "api") (and (eq .process "worker") $worker.enabled $worker.redis.enabled) }}true{{ end -}}
{{- end }}

{{- define "baukit-app.postgresClusterName" -}}
{{- default (printf "%s-postgres" .Release.Name) .Values.backing.postgres.dedicated.clusterName | trunc 50 | trimSuffix "-" -}}
{{- end }}

{{- define "baukit-app.redisKeyNamespace" -}}
{{- default (printf "%s-%s:" (include "baukit-app.labelValue" .Values.product) .Values.deploymentEnvironment) .Values.backing.redis.keyNamespace -}}
{{- end }}

{{/* The chart Redis URL for the selected replica count. */}}
{{- define "baukit-app.redisUrl" -}}
{{- if gt (int .Values.redis.replicas) 1 -}}
{{- printf "redis+sentinel://%s-sentinel:26379/mymaster" (include "baukit-app.redisName" .) -}}
{{- else -}}
{{- printf "redis://%s:6379" (include "baukit-app.redisName" .) -}}
{{- end -}}
{{- end }}

{{/* Env entries that point a process at its backing services: (dict "root" $ "process" <name>). */}}
{{- define "baukit-app.backingEnv" -}}
{{- $root := .root -}}
{{- $prefix := include "baukit-app.envPrefix" $root -}}
{{- if eq (include "baukit-app.postgresMode" $root) "dedicated" }}
- name: {{ printf "%s__DATABASE__URL" $prefix }}
  valueFrom:
    secretKeyRef:
      name: {{ printf "%s-app" (include "baukit-app.postgresClusterName" $root) }}
      key: uri
{{- end }}
{{- if and (include "baukit-app.redisClient" .) (eq (include "baukit-app.redisMode" $root) "dedicated") }}
- name: {{ printf "%s__RATE_LIMIT__REDIS_URL" $prefix }}
  value: {{ include "baukit-app.redisUrl" $root | quote }}
{{- end }}
{{- end }}

{{/* Pod labels from podLabels plus the client labels the shared platform services admit. */}}
{{- define "baukit-app.podLabels" -}}
{{- $labels := deepCopy .root.Values.podLabels -}}
{{- if eq (include "baukit-app.postgresMode" .root) "shared" -}}
{{- $_ := set $labels "baukit.dev/postgres-client" "true" -}}
{{- end -}}
{{- if and (include "baukit-app.redisClient" .) (eq (include "baukit-app.redisMode" .root) "shared") -}}
{{- $_ := set $labels "baukit.dev/redis-client" "true" -}}
{{- end -}}
{{- with $labels -}}
{{ toYaml . }}
{{- end -}}
{{- end }}

{{/* Egress from a process to its backing services: (dict "root" $ "process" <name>). */}}
{{- define "baukit-app.backingEgress" -}}
{{- $root := .root -}}
{{- $backing := $root.Values.backing -}}
{{- $postgres := include "baukit-app.postgresMode" $root -}}
{{- if eq $postgres "shared" }}
- to:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ $backing.postgres.shared.namespace }}
      podSelector:
        matchLabels:
          cnpg.io/cluster: {{ $backing.postgres.shared.clusterName }}
  ports:
    - {protocol: TCP, port: 5432}
{{- else if eq $postgres "dedicated" }}
- to:
    - podSelector:
        matchLabels:
          cnpg.io/cluster: {{ include "baukit-app.postgresClusterName" $root }}
          cnpg.io/podRole: instance
  ports:
    - {protocol: TCP, port: 5432}
{{- end }}
{{- if and (include "baukit-app.redisClient" .) (eq (include "baukit-app.redisMode" $root) "shared") }}
- to:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: {{ $backing.redis.shared.namespace }}
      podSelector:
        matchLabels:
          app.kubernetes.io/name: redis
  ports:
    - {protocol: TCP, port: 26379}
    - {protocol: TCP, port: 6379}
{{- end }}
{{- end }}
