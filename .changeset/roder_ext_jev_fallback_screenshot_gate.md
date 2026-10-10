---
roder-ext-jev: patch
---

# Jev fallback offers its screenshot tool only where the model can see the result

The Jev fallback offers its screenshot tool only to a model whose engine forwards tool-result images (`tool_result_image_input`), not merely one that takes user images, and a model without the tool is no longer told to use one.
