# simple-llm-proxy

A super-lightweight LLM proxy for OpenAI-compatible providers. Only supports the chat completion & models endpoints.

## But why?

I find myself sourcing tokens from three providers consistently:

* Locally (via [llama.cpp](https://github.com/ggml-org/llama.cpp))
* [Amazon Bedrock](https://aws.amazon.com/bedrock/)
* [OpenRouter](https://openrouter.ai/)

Rather than configuring all 3 across all the various clients I use, I'd rather just point my clients at a single proxy running on a homelab server. It also means the API keys stay in one place.

## Alternatives

Like my other public Rust projects, this is just a toy. I'm still very much a Rust noob.

Other, bigger, and better projects that do the same thing and more:

* [LiteLLM](https://github.com/BerriAI/litellm)
* [Bifrost](https://github.com/maximhq/bifrost)

(and this list, of course, is not exhaustive. They're just the projects I'm familiar with.)

They're a bit *too* heavy for my homelab needs, which is why I've tried to move on. (Starting with cobbled-together FastAPI-based Python projects &mdash; see my other repos.)

## To Do

* [x] Connection timeout
* [x] Response timeout
* [x] Config file
* [x] Multiple providers
* [x] "Virtual models" architecture (where each model presented to the client maps to a specific one from a specific provider)
* [x] Optional bearer token authorization for clients
* [ ] Dynamic API keys for providers, by calling a program. Out of scope? Might be used to pull keys from a vault or for generating short-term keys for AWS Bedrock (would be nice to be able to use Roles Anywhere).
* [x] Dynamic mapping of models. Out of scope? Something like "map all models from this provider by adding a prefix to their model names"
* [ ] Better HTTP status responses (e.g. 502, 503, 504 may be applicable for some cases, rather than the generic 500)
* [ ] Basic retry with backoff and maybe jitter when contacting providers (e.g. for some 4xx's, like 408, 429, maybe 425, and most 5xx's mentioned above)
* [ ] Basic Responses API proxying. "Model" will need to be remapped (both between virtual & real and vice-versa) from endpoints such as: `POST /responses`, `GET /responses/xxx`, `POST /responses/input_tokens`, `POST /responses/xxx/cancel`, `POST /responses/compact`
