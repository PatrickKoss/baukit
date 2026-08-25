# Kyverno

This optional base installs Kyverno chart `3.8.2` (Kyverno `v1.18.2`) in the
`kyverno` namespace with one resource-bounded replica of each controller. It
also installs three cluster policies: require CPU/memory requests and limits,
reject explicit `:latest` container image tags, and prevent public
NodePort/LoadBalancer Services or Ingresses from exposing the baukit private
operations range (`9000-9999`, conventionally `9090`) or ports named `ops` or
`metrics`. Resource and image rules cover both regular and init containers.

All policies start with `validationFailureAction: Audit`. Admission policy is
easy to make accidentally disruptive, especially for third-party charts; an
overlay should inspect policy reports, add narrowly justified exclusions, and
patch individual policies to `Enforce` only after the cluster is clean.

The base needs no secrets or identity data. Overlays may set a
`priorityClassName`, enable ServiceMonitors after the observability CRDs exist,
and patch enforcement/exclusions.
