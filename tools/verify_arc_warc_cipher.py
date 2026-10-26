#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Reproduce WARC table-cipher vectors from the pinned original C++ source.

No assets or keys are copied. Only the table-sized branch is executed; entry
key/image branches throw in the harness. Negative floating-to-unsigned casts in
upstream C++ have undefined behavior, so these portable vectors use a >= 0.
"""
from __future__ import annotations
import argparse
from pathlib import Path
import subprocess
import tempfile

PIN = "456834ecf2e5686813802c37efd829310485c57d"
EXPECTED = ["0,0,385e260ba3ed9535", "31,3,b829a59d60a16612", "17,7,27459897321a8a65"]


def block(source: str, signature: str) -> str:
    start = source.index(signature)
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        if source[end] == "{":
            depth += 1
        elif source[end] == "}":
            depth -= 1
        end += 1
    return source[start:end]


def oracle(source: str) -> str:
    header = r'''
#include <cstdint>
#include <cmath>
#include <vector>
#include <string>
#include <stdexcept>
#include <iostream>
using u8=uint8_t;using s8=int8_t;using u32=uint32_t;using u64=uint64_t;using bstr=std::vector<u8>;
bstr operator""_b(const char*s,size_t n){return bstr(s,s+n);}
struct Plugin{size_t entry_name_size=32;bstr logo_data;};
namespace err{struct NotSupportedError:std::runtime_error{using runtime_error::runtime_error;};}
namespace algo{struct Range{size_t a,b;struct It{size_t n;size_t operator*()const{return n;}void operator++(){++n;}bool operator!=(It o)const{return n!=o.n;}};It begin()const{return{a};}It end()const{return{b};}};Range range(size_t a,size_t b){return{a,b};}}
static const double pi=3.14159265358979323846;
static u64 get_essential_key_pos_addend1(u64){throw std::runtime_error("entry branch outside oracle");}
static void decrypt_initial_160plus(const Plugin&,bstr&){throw std::runtime_error("entry branch outside oracle");}
'''
    declarations = block(source, "class CustomLcg final") + ";\n"
    begin = source.index("static const bstr essential_crypt_key")
    declarations += source[begin:source.index(";", begin)+1] + "\n"
    for signature in ["CustomLcg::CustomLcg", "void CustomLcg::reset", "void CustomLcg::step", "static size_t get_max_table_size", "static double get_lcg_xor", "static u64 get_essential_key_pos_addend2", "void warc::decrypt_essential"]:
        declarations += block(source, signature).replace("void warc::decrypt_essential", "void decrypt_essential") + "\n"
    driver = r'''
int main(){for(auto p:std::vector<std::pair<int,int>>{{0,0},{31,3},{17,7}}){Plugin plugin;bstr data((32+24)*16384);data[0]=p.first;data[1]=p.second;decrypt_essential(plugin,170,data);uint64_t hash=14695981039346656037ull;for(auto b:data){hash^=b;hash*=1099511628211ull;}std::cout<<std::dec<<p.first<<","<<p.second<<","<<std::hex<<hash<<"\n";}}
'''
    return header + declarations + driver


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--compiler", default="clang++")
    args = parser.parse_args()
    checkout = args.source.resolve()
    actual = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
    if actual != PIN:
        raise SystemExit(f"pin mismatch: {actual}")
    if subprocess.check_output(["git", "-C", str(checkout), "status", "--porcelain"], text=True).strip():
        raise SystemExit("source checkout must be clean")
    source = (checkout / "src/dec/shiina_rio/warc/decrypt.cc").read_text(encoding="utf-8-sig")
    with tempfile.TemporaryDirectory(prefix="crysirror-warc-") as temporary:
        directory = Path(temporary)
        cpp, executable = directory / "oracle.cpp", directory / "oracle"
        cpp.write_text(oracle(source))
        subprocess.run([args.compiler, "-std=c++14", "-O2", str(cpp), "-o", str(executable)], check=True)
        actual = subprocess.check_output([str(executable)], text=True).splitlines()
    if actual != EXPECTED:
        raise SystemExit(f"WARC table-cipher oracle differs: {actual!r}")
    print(f"{PIN}: original C++ table-cipher oracle, 3 vectors passed")
    print("\n".join(actual))


if __name__ == "__main__":
    main()
