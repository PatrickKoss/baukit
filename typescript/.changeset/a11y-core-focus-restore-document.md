---
'@baukit/a11y-core': patch
---

`useFocusTrap` no longer throws when the document goes away before its deferred focus restore runs, for example after a Jest test resets `globalThis.document`.
