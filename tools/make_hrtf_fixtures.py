#!/usr/bin/env python3
"""Generate authored netCDF4 SOFA impulses, without measured HRIR data.

Manual regeneration: uv run --with netCDF4 --with h5py tools/make_hrtf_fixtures.py
The generator dependency is not needed by Rust tests.
"""
from pathlib import Path
import argparse


def generate(directory: Path) -> None:
    import netCDF4
    import numpy as np
    directory.mkdir(parents=True, exist_ok=True)
    for filename, swapped, units in [
        ("impulse-v3.sofa", False, "meter"),
        ("missing-type.sofa", False, "meter"),
        ("impulse.sofa", False, "meter"),
        ("swapped-ears.sofa", True, "meter"),
        ("unsupported-units.sofa", False, "centimeter"),
    ]:
        if filename == "impulse-v3.sofa":
            import h5py
            with h5py.File(directory / filename, "w", libver=("v110", "v110"), track_order=True):
                pass
        with netCDF4.Dataset(directory / filename, "a" if filename == "impulse-v3.sofa" else "w", format="NETCDF4") as file:
            for key, value in {"Conventions": "SOFA", "SOFAConventions": "SimpleFreeFieldHRIR", "DataType": "FIR"}.items():
                file.setncattr(key, value)
            for key, count in {"I": 1, "C": 3, "R": 2, "E": 1, "N": 8, "M": 2}.items():
                file.createDimension(key, count)
            def array(name, dimensions, values, kind=None, unit=None):
                value = file.createVariable(name, "f8", dimensions)
                value[:] = np.asarray(values, dtype="<f8")
                if kind:
                    value.setncattr("Type", kind)
                if unit:
                    value.setncattr("Units", unit)
                return value
            array("SourcePosition", ("M", "C"), [[1, 0, 0], [0, -1, 0]], None if filename == "missing-type.sofa" else "cartesian", units)
            receivers = [[[0], [.09], [0]], [[0], [-.09], [0]]]
            if swapped:
                receivers.reverse()
            array("ReceiverPosition", ("R", "C", "I"), receivers, "cartesian", "meter")
            array("ListenerPosition", ("I", "C"), [[0, 0, 0]], "cartesian", "meter")
            array("EmitterPosition", ("E", "C", "I"), [[[0], [0], [0]]], "cartesian", "meter")
            array("ListenerView", ("I", "C"), [[1, 0, 0]], "cartesian")
            array("ListenerUp", ("I", "C"), [[0, 0, 1]], "cartesian")
            array("Data.SamplingRate", ("I",), [48000], unit="hertz")
            impulse = np.zeros((2, 2, 8), dtype="<f8")
            impulse[:, 0, 0] = 1
            impulse[:, 1, 0] = .5
            delay = [[3.1, 5.1]]
            if swapped:
                impulse = impulse[:, ::-1, :]
                delay = [[5.1, 3.1]]
            array("Data.IR", ("M", "R", "N"), impulse)
            array("Data.Delay", ("I", "R"), delay)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=Path(__file__).resolve().parents[1] / "crates/crysirror-acoustics/tests/fixtures")
    args = parser.parse_args()
    generate(args.output_dir)


if __name__ == "__main__":
    main()
