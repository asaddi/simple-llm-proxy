# simple-llm-proxy

A super-lightweight LLM proxy for OpenAI-compatible providers. Only supports the chat completion & models endpoints.

## To Do

* [x] Connection timeout
* [x] Response timeout
* [x] Config file
* [x] Multiple providers
* [x] "Virtual models" architecture (where each model presented to the client maps to a specific one from a specific provider)
* [x] Optional bearer token authorization for clients
* [_] Dynamic API keys for providers, by calling a program. Out of scope? Might be used to pull keys from a vault or for generating short-term keys for AWS Bedrock.
* [_] Dynamic mapping of models. Out of scope? Something like "map all models from this provider by adding a prefix to their model names"
