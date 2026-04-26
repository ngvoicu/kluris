"""Kluris pack — Docker chat server for brains.

Package-relative imports throughout (``from .config import Config``)
because the same source is consumed two ways:

- In source / tests: ``from kluris.pack.X import Y``
- In the packed Docker image: ``from app.X import Y``

A forbidden-import test in ``tests/pack/test_pack_stager.py`` enforces
that nothing under :mod:`kluris.pack` uses absolute ``kluris.pack.*``
or ``kluris.core.*`` imports, since either form would break the
relativity contract.
"""
