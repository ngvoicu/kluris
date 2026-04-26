"""Read-only kluris brain runtime.

A deliberately tiny package that contains only the read-side primitives
the packed Docker chat server needs: frontmatter reading, neuron
indexing/sandboxing, deprecation diagnostics, lexical search, wake-up
payload building, and neuron excerpt extraction.

Has zero dependency on Click, Rich, git, or anything else in
``kluris.core``. The Docker stager copies this package as-is into the
image; the CLI wraps it via thin re-exports in ``kluris.core.*``.
"""
