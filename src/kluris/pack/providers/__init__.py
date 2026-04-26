"""LLM provider abstractions for the kluris pack chat server.

The :mod:`kluris.pack.providers.base` module defines the
:class:`LLMProvider` ABC. Concrete implementations live in
:mod:`kluris.pack.providers.apikey` (Anthropic + OpenAI HTTP shapes)
and :mod:`kluris.pack.providers.oauth` (OAuth 2.0 client_credentials
fronting an OpenAI-compatible API).
"""
