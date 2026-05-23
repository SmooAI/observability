---
'@smooai/observability': patch
---

fix(node): re-export `setGenAIAttributes` / `recordGenAIMessage` / GenAI types from node entry — were missing in 0.10.0, broke backend builds importing the helpers from the bare package name
