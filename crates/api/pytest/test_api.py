"""Tests for the `philis` Python object surface (the Tier-0 subset).

The Python bindings are a deliberate strict subset of the Rust surface: load a
PDK, load a circuit, run, read counters. No builder, no Layout, no constraints —
those live in the Rust crate (see `STUBS.md` and the API design docs).

Build first: `maturin develop --features extension-module`.
"""

import pytest

philis = pytest.importorskip(
    "philis",
    reason="build the extension first: `maturin develop --features extension-module`",
)

SPICE = """\
.subckt inv a y
m1 y a 0 0 nmos
m2 y a vdd vdd pmos
.ends
"""


@pytest.fixture
def pdk(tmp_path):
    p = tmp_path / "pdk.json"
    p.write_text('{"tech": "stub"}')
    return philis.Pdk.from_file(str(p))


def test_classes_are_exported():
    assert hasattr(philis, "Pdk")
    assert hasattr(philis, "Circuit")
    assert hasattr(philis, "RunResult")


def test_pdk_from_file(tmp_path):
    p = tmp_path / "pdk.json"
    p.write_text('{"tech": "stub"}')
    pdk = philis.Pdk.from_file(str(p))
    assert pdk.warnings() == []


def test_pdk_from_file_missing_raises():
    with pytest.raises(ValueError):
        philis.Pdk.from_file("/no/such/file.json")


def test_pdk_from_env(tmp_path, monkeypatch):
    p = tmp_path / "pdk.json"
    p.write_text('{"tech": "stub"}')
    monkeypatch.setenv("PHILIS_PYTEST_PDK", str(p))
    pdk = philis.Pdk.from_env("PHILIS_PYTEST_PDK")
    assert pdk.warnings() == []
    with pytest.raises(ValueError):
        philis.Pdk.from_env("PHILIS_PYTEST_PDK_UNSET")


def test_circuit_from_text_and_run(pdk):
    ckt = philis.Circuit.from_spice_text(SPICE)
    result = ckt.run(pdk)
    assert result.device_count >= 1
    assert result.placed_count == result.device_count
    assert result.routed_segments == 0  # stub router


def test_circuit_from_file(tmp_path, pdk):
    f = tmp_path / "inv.sp"
    f.write_text("m1 d g s b nmos\nm2 d g s b nmos\n")
    ckt = philis.Circuit.from_spice_file(str(f))
    result = ckt.run(pdk)
    assert result.device_count == 2


def test_run_result_fields_are_readonly(pdk):
    result = philis.Circuit.from_spice_text("m1 d g s b nmos\n").run(pdk)
    with pytest.raises(AttributeError):
        result.device_count = 99
