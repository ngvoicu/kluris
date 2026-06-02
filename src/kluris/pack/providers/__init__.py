"""LLM provider abstractions for the kluris pack chat server.

The :mod:`kluris.pack.providers.base` module defines the
:class:`LLMProvider` ABC. The single concrete implementation lives in
:mod:`kluris.pack.providers.litellm_provider`
(:class:`LiteLLMProvider`), which routes Anthropic api-key, OpenAI
api-key (Chat Completions or Responses API), and OAuth gateways through
LiteLLM model strings computed in :mod:`kluris.pack.config`.
"""
