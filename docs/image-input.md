# Images handed to a model

Toad Agent prepares desktop drops and uploaded phone attachments through the
same path in `driver/rig/images.rs`. The original file stays untouched. The
model receives its path and, when decoding succeeds, normalized image bytes.
An unreadable, corrupt, unsupported, or over-budget image stays a path with
an explanation. Its bytes never become persistent model input.

The limits are local budgets, not a promise that every model accepts images:

- Read at most 20 MiB from a regular attachment file. Decode at most 40 million
  pixels, with a 16,000-pixel bound on either dimension and a 160 MiB decoder
  allocation limit. Two blocking workers may decode at once; cancellation
  does not release their permits until the workers actually finish.
- Honor orientation, flatten alpha onto white, resize to at most 2,000 pixels
  on either edge, and encode JPEG at quality 85. Reduce dimensions further
  until the JPEG is at most 1 MiB. The MIME type comes from this output,
  never the attachment's declaration.
- Admit four image attachments per message. Bound replay to four recent image
  blocks, including tool images, and roughly 5.34 MiB of base64 image data.
  Groq uses three blocks because its current documented models differ in count
  limits. Older pixels become explicit placeholders; file paths and tool
  call/result pairing remain. Text, tools and image tokens must still fit context.
- Validate and normalize inline tool images too. An external tool image URL
  is not fetched automatically. ACP attachments remain resource links owned
  by the external harness.
- HEIC is not decoded by the bundled image library. Keep it as a path and
  ask for JPEG conversion; do not mislabel its bytes as another format. GIF
  and WebP are decoded as still images. Re-encoding discards metadata.

Phone-side downscaling is not required for correctness. The current upload
protocol's separate four-file/10-MiB-per-file limits still apply; the core
normalizes attachments after upload. The upload protocol remains unchanged.

## Provider references

Checked 2026-09-14. Provider, model, endpoint and account limits differ. These
are source observations, not live-provider acceptance results. Unknown limits
are left unknown, especially for subscription backends and custom endpoints.

| Connection | Documented input and limits | Source |
| --- | --- | --- |
| Anthropic | JPEG, PNG, GIF, WebP; base64, URL or file reference. Direct API: 10 MB base64 per image, 32 MB request; 8,000-pixel edges, with stricter many-image rules. Count depends on context tier (100 or 600). | [Vision](https://platform.claude.com/docs/en/build-with-claude/vision) |
| OpenAI Chat and Responses | PNG, JPEG, WebP, nonanimated GIF; URL or base64, and file IDs on Responses. Current guide lists 512 MB total payload and 1,500 images; model/detail patch and context budgets apply separately. | [Images and vision](https://developers.openai.com/api/docs/guides/images-vision) |
| ChatGPT backend | Rig owns the subscription transport. Public API limits above are not a verified contract for this backend. Use the local budget and report refusal honestly. | Installed Rig `providers/chatgpt` adapter |
| OpenRouter | Base64 or URL image input routes to the selected model/provider. No universal downstream byte/count guarantee is assumed. | [Image inputs](https://openrouter.ai/docs/guides/overview/multimodal/image-understanding) |
| xAI | JPEG/PNG; 20 MiB per image; no fixed image-count ceiling in the guide. The context budget still applies. | [Image understanding](https://docs.x.ai/developers/model-capabilities/images/understanding) |
| Gemini generateContent | Inline image bytes share a 20 MB request budget with text and instructions. Files API is recommended for larger/reused inputs; Toad uses inline input. | [Image understanding](https://ai.google.dev/gemini-api/docs/generate-content/image-understanding) |
| Mistral | PNG, JPEG, GIF, WebP; 20 MB per image; count depends on the model and token budget. | [Known limitations](https://docs.mistral.ai/resources/known-limitations) |
| Groq | Base64 data URL or remote URL. The current vision guide lists 20 MB URL requests, with five images for Qwen 3.6 and three for Qwen 3.8. Other model limits must be checked independently. | [Vision](https://console.groq.com/docs/vision) |
| DeepSeek | The vision guide documents JPEG, PNG, GIF, WebP and a 48 MiB request-body limit including base64. Vision is model-specific; a text model does not gain vision through normalization. | [Vision](https://api-docs.deepseek.com/guides/vision/) |
| Z.ai | Vision models are separate from text models. A universal numeric limit was not verified in the reviewed primary guide; use bounded JPEG input and preserve provider errors. | [GLM-4.5V](https://docs.z.ai/guides/vlm/glm-4.5v) |
| Copilot | Vision support, MIME types and limits are per model; SDK capability metadata describes them. The SDK documentation does not establish a universal quota for Rig's subscription transport. | [Image input](https://docs.github.com/en/copilot/how-tos/copilot-sdk/features/image-input) |
| Ollama | Vision models accept image bytes; native REST uses base64. Capacity depends on the selected model and server. | [Vision](https://docs.ollama.com/capabilities/vision) |
| Custom OpenAI-compatible | The operator's endpoint defines its limits and supported models. Compatibility names a wire format, not a common quota or vision guarantee. | Endpoint configuration in Settings |

## Verification

Tests cover corrupt input, a generated 4,000 × 3,000 fixture, alpha, declared-MIME
mismatch, JPEG budgets, and cancellation. The wire harness uses real Rig Chat
and Responses adapters against disposable HTTP providers. A real phone photo
on three live providers remains a release check; these fixtures do not claim
to establish live vendor acceptance or phone picker behavior.
