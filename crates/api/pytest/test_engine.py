"""Example tests for the `philis` Python extension module.

Build the module into the active environment before running these, e.g.:

    maturin develop --features extension-module   # debug build into the venv
    pytest                                         # runs from the repo root

`pyproject.toml` already points pytest at this directory and tells maturin to
build with the `extension-module` feature.
"""

import pytest

philis = pytest.importorskip(
    "philis",
    reason="build the extension first: `maturin develop --features extension-module`",
)


def test_module_has_version():
    assert isinstance(philis.__version__, str)
    assert philis.__version__ == "1.0.0"


def test_place_and_route_basic():
    out = philis.place_and_route("a 0 0\nb 3 4")
    assert out == "placed 2 cells, bbox 3x4, hpwl 7"


def test_place_and_route_skips_comments_and_blanks():
    out = philis.place_and_route("# header\ninv0 0 0\n\ninv1 10 5\n")
    assert out == "placed 2 cells, bbox 10x5, hpwl 15"


def test_empty_input():
    assert philis.place_and_route("\n# nothing here\n") == "placed 0 cells"


def test_malformed_input_raises_value_error():
    with pytest.raises(ValueError):
        philis.place_and_route("a 1")  # missing y coordinate
