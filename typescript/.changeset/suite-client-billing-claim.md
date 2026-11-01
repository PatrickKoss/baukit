---
'@baukit/suite-client': minor
---

Breaking: `verifyTicket` accepts the optional `billing` claim from Schlüsselbund C18 and returns it as `TicketClaims.billing` with `BillingChannel` and `BillingClaim`. A `null` claim, unknown fields, `soldBy` outside `app_store` and `play_store`, and `manageUrl` outside `web` or without an HTTP(S) host fail as `ticket_invalid`. Code that builds `TicketClaims` literals needs the new optional field in mind.
