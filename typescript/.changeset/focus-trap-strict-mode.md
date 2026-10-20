---
'@baukit/a11y-core': patch
---

Keep the original overlay opener across StrictMode effect replays and focus ref changes so closing
the overlay restores focus to its trigger. Capture a new opener for each separate opening.
