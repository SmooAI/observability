---
'@smooai/observability': minor
---

Instrument the OpenAI Node SDK, and stop leaking prompt content into GenAI span events.

`wrapOpenAI(client, options)` returns a proxy of an OpenAI client whose `chat.completions.create` emits OTel GenAI semantic-convention spans — request model / sampling params / tool names on the way out, response model, id, finish reason, and token usage on the way back. Streaming is handled: the span stays open until the stream drains (or the consumer breaks out early), and picks up the usage chunk emitted under `stream_options.include_usage`. The client is duck-typed, so this adds no dependency on `openai` and works against Groq, Together, Fireworks, DeepSeek, Azure OpenAI, and any OpenAI-compatible gateway via `{ system: 'groq' }`.

Cost has a seam now: nothing in the platform computes an LLM price on its own, which is why the dashboard's cost column is empty. Pass `costUsd({ requestModel, responseModel, inputTokens, outputTokens, cachedTokens })` and it lands on `gen_ai.usage.cost_usd`.

`recordGenAIMessage` now routes content through the SDK's PII scrub before it leaves the process. Prompts are the most PII-dense payload this SDK can touch, and `wrapOpenAI` keeps content recording **off** by default (`{ recordContent: true }` opts in).
