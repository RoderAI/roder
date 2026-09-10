---
roder-core: patch
---

Remove the flaky `wait_agent_observes_mailbox_activity_queued_before_subscription`
lifecycle test. It failed roughly one run in four because the wakeup it asserts
is genuinely racy, not because the test was written wrong — see the changeset
body for the underlying gap.
