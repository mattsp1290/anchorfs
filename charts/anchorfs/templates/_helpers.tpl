{{/*
AnchorFS chart helpers: image-by-digest validation, labels, security snippets.
Only Helm 3.13-compatible (Sprig) functions are used.
*/}}

{{/* Usage: include "anchorfs.image" (dict "img" .Values.image.service "path" "image.service") */}}
{{- define "anchorfs.image" -}}
{{- $r := required (printf "%s.repository is required" .path) .img.repository -}}
{{- $d := required (printf "%s.digest is required" .path) .img.digest -}}
{{- if not (regexMatch "^sha256:[a-f0-9]{64}$" $d) -}}
{{- fail (printf "%s.digest must match sha256:<64 lowercase hex>, got %q" .path $d) -}}
{{- end -}}
{{- printf "%s@%s" $r $d -}}
{{- end -}}

{{- define "anchorfs.selectorLabels" -}}
app.kubernetes.io/name: anchorfs
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "anchorfs.labels" -}}
{{ include "anchorfs.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{/* Dev-dependency labels: same name/instance pair plus a component. Usage: include "anchorfs.devLabels" (dict "ctx" . "component" "dev-postgres") */}}
{{- define "anchorfs.devLabels" -}}
{{ include "anchorfs.labels" .ctx }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{/* Container port parsed from listenAddr (host:port). */}}
{{- define "anchorfs.containerPort" -}}
{{- regexFind "[0-9]+$" .Values.listenAddr | int -}}
{{- end -}}

{{/* Validated deployment id and durability profile. */}}
{{- define "anchorfs.deploymentId" -}}
{{- if not (regexMatch "^[a-z0-9-]{1,64}$" .Values.deploymentId) -}}
{{- fail "deploymentId must match ^[a-z0-9-]{1,64}$" -}}
{{- end -}}
{{- .Values.deploymentId -}}
{{- end -}}

{{- define "anchorfs.durability" -}}
{{- if not (has .Values.durabilityProfile (list "durable" "development")) -}}
{{- fail "durabilityProfile must be durable or development" -}}
{{- end -}}
{{- .Values.durabilityProfile -}}
{{- end -}}

{{/* Unprivileged container security context (service, test, worker). */}}
{{- define "anchorfs.restrictedContext" -}}
allowPrivilegeEscalation: false
readOnlyRootFilesystem: true
runAsNonRoot: true
runAsUser: 10001
runAsGroup: 10001
capabilities:
  drop: ["ALL"]
{{- end -}}

{{- define "anchorfs.podContext" -}}
runAsNonRoot: true
runAsUser: 10001
runAsGroup: 10001
fsGroup: 10001
seccompProfile:
  type: RuntimeDefault
{{- end -}}
