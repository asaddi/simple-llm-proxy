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

* [Bifrost](https://github.com/maximhq/bifrost)
* [LiteLLM](https://github.com/BerriAI/litellm)

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
* [x] Basic retry with backoff ~~and maybe jitter~~ when contacting providers (e.g. for some 4xx's, like 408, 429, maybe 425, and most 5xx's mentioned above)
   * Will implement jitter another time/if needed.
* [ ] Basic Responses API proxying. ~~"Model" will need to be remapped~~ (both between virtual & real and vice-versa) from endpoints such as: `POST /responses`, `GET /responses/xxx`, `POST /responses/input_tokens`, `POST /responses/xxx/cancel`, `POST /responses/compact`
   * `POST /responses` is already supported, which is enough for backends that don't store responses (e.g. llama.cpp)
   * It turns out that even `POST /chat/completions` returns the actual model that was used. I don't do any remapping of the returned model there and clients seem to work fine. So I will KISS and forget about remapping the returned model.
   * Frankly, I personally don't have any frontends or agents that support Responses API, so I don't have much motivation to implement this.
