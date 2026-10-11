---
roder-ext-openai-responses: patch
---

# Honor the rate-limit retry policy for OpenRouter credit holds

Use the configured HTTP 429 retry eligibility for the recognized temporary HTTP 402 in-flight spending hold. Wait for Retry-After and resend identical inference input; permanent credit failures do not retry.
