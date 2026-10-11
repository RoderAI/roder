---
roder-ext-openai-responses: patch
---

# Recover temporary OpenRouter in-flight credit limits

Classify the structured HTTP 402 in-flight budget limit as retryable and retain Retry-After advice. Requests exceeding the whole credit budget, exhausted credits, and unrecognized payment failures remain non-retryable quota failures.
