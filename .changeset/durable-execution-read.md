---
roder-app-server: patch
roder-core: patch
roder-ext-jsonl-thread-store: patch
---

Recover external-tool execution status from durable thread history after a host restart. Unresolved requests report an uncertain outcome and cannot be replayed or resolved by a replacement executor.

Persist external execution requests before delivery and resolution receipts before acknowledgement. Persistence failures prevent delivery or successful acknowledgement.

Flush JSONL event writes before append returns so immediate process exit cannot drop a delivered execution receipt.
