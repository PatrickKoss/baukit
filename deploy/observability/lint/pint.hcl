rule {
  match {
    kind = "alerting"
  }
  label "severity" {
    severity = "bug"
    value    = "(critical|warning|info)"
    required = true
  }
  annotation "summary" {
    severity = "bug"
    required = true
  }
  annotation "description" {
    severity = "bug"
    required = true
  }
  annotation "runbook_url" {
    severity = "bug"
    required = true
  }
}

rule {
  match {
    path = "(.*/)?observability-platform/(alerts|recording-rules)/.*"
    kind = "alerting"
  }
  label "component" {
    severity = "bug"
    required = true
  }
}
