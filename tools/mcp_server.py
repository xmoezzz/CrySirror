#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Local stdio MCP project tools. No arbitrary shell execution or HTTP listener.

This controls project data and invokes the desktop host for captures. It is not
an in-process connection to a running native game. Writes need --allow-write.
"""
from __future__ import annotations
import argparse
import base64
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid
from desktop_project import load, validate, asset_inventory, require, logical_path
from package_game import package

PROTOCOL = "2025-06-18"
MAX_MESSAGE = 8 * 1024 * 1024
EMPTY = {"type": "object", "properties": {}, "additionalProperties": False}
FRAME_PROPERTIES = {"frames":{"type":"integer","minimum":1,"maximum":3600},
                    "step_ms":{"type":"integer","minimum":0,"maximum":60000},
                    "press_at":{"type":"array","maxItems":3600,"uniqueItems":True,"items":{"type":"integer","minimum":0,"maximum":3599}}}

def frame_arguments(args):
    frames=args.get("frames",1);step=args.get("step_ms",16);press=args.get("press_at",[])
    require(type(frames) is int and 1<=frames<=3600,"frames must be 1..3600")
    require(type(step) is int and 0<=step<=60000,"step_ms must be 0..60000")
    require(isinstance(press,list) and len(press)<=3600 and all(type(i) is int and 0<=i<frames for i in press) and len(set(press))==len(press),"press_at must contain unique frame indices")
    result=["--frames",str(frames),"--step-ms",str(step)]
    for i in press:result += ["--press-at",str(i)]
    return result


def text(value) -> dict:
    return {"type": "text", "text": json.dumps(value, ensure_ascii=False, allow_nan=False)}

class Server:
    def __init__(self, project: Path, executable: Path | None = None, *, allow_write: bool = False,
                 output_dir: Path | None = None):
        self.project = project.resolve(strict=True)
        load(self.project)
        self.executable = executable.resolve(strict=True) if executable else None
        self.allow_write = allow_write
        self.output_dir = (output_dir or self.project.parent / ".crysirror-mcp").resolve()
        self.initialized = False
        self.ready = False

    def tools(self) -> list[dict]:
        result = [
            {"name": "project_inspect", "description": "Read the project description; reports native adapter gaps.", "inputSchema": EMPTY, "annotations": {"readOnlyHint": True}},
            {"name": "project_validate", "description": "Validate project data and explicit asset paths, not native game compatibility.", "inputSchema": EMPTY, "annotations": {"readOnlyHint": True}},
        ]
        if self.allow_write:
            result += [{"name": "project_replace", "description": "Replace this selected project's data after full validation. No other files are editable. Compare expected_sha256 to avoid lost updates.",
                        "inputSchema": {"type": "object", "properties": {"project": {"type": "object"}, "expected_sha256": {"type": "string", "pattern": "^[a-f0-9]{64}$"}}, "required": ["project", "expected_sha256"], "additionalProperties": False}, "annotations": {"readOnlyHint": False, "destructiveHint": True}}]
        if self.executable:
            result += [{"name":"shader_validate","description":"Compile a client-scoped WGSL surface asset against ABI v1 and all color/depth/shadow stages without a GPU.","inputSchema":{"type":"object","properties":{"client":{"type":"string"},"asset":{"type":"string"}},"required":["client","asset"],"additionalProperties":False},"annotations":{"readOnlyHint":True}},
                       {"name": "scene_capture", "description": "Run the configured host's real GPU renderer and return PNG pixels. Each call starts from project data, not a live game session.",
                        "inputSchema": {"type":"object","properties":dict(FRAME_PROPERTIES,renderer={"type":"string","enum":["raster","compute-raytrace"]}),"additionalProperties":False}},
                       {"name":"engine_trace","description":"Execute deterministic fresh-session engine/VM ticks and inspect World, UI events and service state without a GPU. press_at sends Enter to the focused UI or VM.","inputSchema":{"type":"object","properties":FRAME_PROPERTIES,"additionalProperties":False}},
                       {"name": "game_package", "description": "Package this project and the configured, already-built executable. Does not compile/sign or test native VMs.", "inputSchema": EMPTY}]
        return result

    def call(self, name: str, args: dict) -> dict:
        require(isinstance(args, dict), "arguments must be an object")
        require(name in {t["name"] for t in self.tools()}, "tool unavailable or permission not granted")
        p = load(self.project)
        if name in ("project_inspect", "project_validate", "game_package"):
            require(not args, "unexpected arguments")
        if name == "project_inspect":
            import hashlib
            value = {"project": p, "sha256": hashlib.sha256(self.project.read_bytes()).hexdigest(),
                     "registered_native_adapters": None, "registry_verified": False, "source_native_adapters": ["bgi", "rfvp", "siglus"], "builtin_adapter": "crysirror-script-v1"}
            if self.executable:
                run = subprocess.run([str(self.executable), "inspect", str(self.project)], cwd=self.project.parent, capture_output=True, timeout=30, check=False)
                require(run.returncode == 0, "host inspection failed: " + run.stderr[-16384:].decode(errors="replace"))
                require(len(run.stdout) <= MAX_MESSAGE, "host inspection exceeds response budget")
                host = json.loads(run.stdout)
                adapters = host.get("registered_adapters")
                require(isinstance(adapters, list) and all(isinstance(a, str) for a in adapters), "host registry missing")
                value["registered_native_adapters"] = [a for a in adapters if a != "crysirror-script-v1"]
                value["registry_verified"] = True
            return {"content": [text(value)]}
        if name == "project_validate":
            files = asset_inventory(self.project, p)
            return {"content": [text({"schema_valid": True, "asset_files": len(files), "asset_scope": "declared", "native_vms_verified": False})]}
        if name == "project_replace":
            import hashlib
            require(set(args) == {"project", "expected_sha256"}, "replacement requires project and expected_sha256")
            before = self.project.read_bytes()
            require(hashlib.sha256(before).hexdigest() == args["expected_sha256"], "project changed since inspection; reload before replacing")
            replacement = validate(copy.deepcopy(args["project"]))
            data = (json.dumps(replacement, indent=2, ensure_ascii=False, allow_nan=False)+"\n").encode()
            require(len(data) <= 4*1024*1024, "project exceeds 4 MiB")
            asset_inventory(self.project, replacement)
            fd, name_tmp = tempfile.mkstemp(prefix=".crysirror-project-", dir=self.project.parent)
            try:
                with os.fdopen(fd, "wb") as stream:
                    stream.write(data)
                    stream.flush()
                    os.fsync(stream.fileno())
                # This is a single-writer tool, not a cross-process CAS transaction.
                require(self.project.read_bytes() == before, "concurrent project edit detected")
                os.replace(name_tmp, self.project)
            finally:
                if os.path.exists(name_tmp):
                    os.unlink(name_tmp)
            return {"content": [text({"updated": True, "sha256": hashlib.sha256(data).hexdigest()})]}
        if name=="shader_validate":
            require(set(args)=={"client","asset"},"shader_validate needs client and asset")
            require(isinstance(args["client"],str) and any(c["name"]==args["client"]for c in p["clients"]),"unknown shader client")
            require(logical_path(args["asset"])==args["asset"] and args["asset"].endswith(".wgsl"),"canonical WGSL asset required")
            run=subprocess.run([str(self.executable),"shader-check",str(self.project),"--client",args["client"],"--asset",args["asset"]],cwd=self.project.parent,capture_output=True,timeout=30,check=False)
            require(run.returncode==0,"shader validation failed: "+run.stderr[-16384:].decode(errors="replace"))
            require(len(run.stdout)<=MAX_MESSAGE,"shader validation response too large")
            return {"content":[text(json.loads(run.stdout))]}
        self.output_dir.mkdir(parents=True, exist_ok=True)
        if name == "engine_trace":
            require(set(args)<=set(FRAME_PROPERTIES),"unexpected trace arguments")
            options=frame_arguments(args)
            run=subprocess.run([str(self.executable),"trace",str(self.project),*options,"--data-dir",str(self.output_dir/"state")],cwd=self.project.parent,capture_output=True,timeout=120,check=False)
            require(run.returncode==0,"engine trace failed: "+run.stderr[-16384:].decode(errors="replace"))
            require(len(run.stdout)<=MAX_MESSAGE,"engine trace exceeds response budget")
            return {"content":[text(json.loads(run.stdout))]}
        if name == "scene_capture":
            require(set(args) <= set(FRAME_PROPERTIES)|{"renderer"}, "unexpected capture arguments")
            options=frame_arguments(args)
            renderer=args.get("renderer","raster")
            require(renderer in ("raster","compute-raytrace"),"unsupported renderer")
            frames = args.get("frames", 1)
            require(type(frames) is int and 1 <= frames <= 3600, "frames must be 1..3600")
            output = self.output_dir / (uuid.uuid4().hex + ".png")
            run = subprocess.run([str(self.executable), "render", str(self.project), "--output", str(output),
                                  *options, "--renderer",renderer, "--data-dir", str(self.output_dir / "state")],
                                 cwd=self.project.parent, capture_output=True, timeout=120, check=False)
            require(run.returncode == 0, "GPU capture failed: " + run.stderr[-16384:].decode(errors="replace"))
            require(output.is_file() and output.stat().st_size <= 16*1024*1024, "capture is missing/too large")
            pixels = output.read_bytes()
            require(pixels.startswith(b"\x89PNG\r\n\x1a\n"), "host did not produce PNG")
            return {"content": [text({"path": str(output), "frames": frames,"renderer":renderer}),
                                {"type": "image", "mimeType": "image/png", "data": base64.b64encode(pixels).decode()}]}
        if name == "game_package":
            output = self.output_dir / (uuid.uuid4().hex + ".zip")
            manifest = package(self.project, output, executable=self.executable)
            return {"content": [text({"path": str(output), "manifest": manifest})]}
        raise ValueError("unknown tool")

    def handle(self, message) -> dict | None:
        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0" or not isinstance(message.get("method"), str):
            return {"jsonrpc": "2.0", "id": None, "error": {"code": -32600, "message": "Invalid Request"}}
        request_id = message.get("id")
        notification = "id" not in message
        method = message["method"]
        params = message.get("params", {})
        if notification:
            if method == "notifications/initialized" and self.initialized:
                self.ready = True
            return None
        def result(value):
            return {"jsonrpc": "2.0", "id": request_id, "result": value}
        def error(code, value):
            return {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": value}}
        if type(request_id) not in (int, str):
            return error(-32600, "id must be an integer or string")
        if not isinstance(params, dict):
            return error(-32602, "params must be an object")
        if method == "ping":
            return result({})
        if method == "initialize":
            if self.initialized:
                return error(-32600, "already initialized")
            if not isinstance(params.get("protocolVersion"), str) or not isinstance(params.get("clientInfo"), dict) or not isinstance(params.get("capabilities"), dict):
                return error(-32602, "initialize requires protocolVersion, clientInfo and capabilities")
            self.initialized = True
            return result({"protocolVersion": PROTOCOL, "capabilities": {"tools": {"listChanged": False}},
                           "serverInfo": {"name": "CrySirror-project-tools", "version": "0.2.0"},
                           "instructions": "Local project tools, not a live native VM debugger. Inspect sha256 before replacing data. Native RFVP/Siglus adapters are not linked."})
        if not self.ready:
            return error(-32002, "initialize and notifications/initialized are required")
        if method == "tools/list":
            if params.get("cursor"):
                return error(-32602, "no continuation cursor")
            return result({"tools": self.tools()})
        if method == "tools/call":
            name = params.get("name")
            if not isinstance(name, str) or name not in {t["name"] for t in self.tools()}:
                return error(-32602, "unknown or unavailable tool")
            try:
                return result(self.call(name, params.get("arguments", {})))
            except (ValueError, OSError, TypeError, subprocess.SubprocessError) as exc:
                return result({"content": [text({"error": str(exc)})], "isError": True})
        return error(-32601, "Method not found")

    def serve(self, stdin, stdout) -> None:
        while True:
            line = stdin.readline(MAX_MESSAGE + 1)
            if not line:
                return
            if len(line) > MAX_MESSAGE:
                # Terminate rather than interpreting fragments of an oversized message.
                print("MCP message exceeds 8 MiB", file=sys.stderr)
                return
            try:
                message = json.loads(line, parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))
                response = self.handle(message)
            except (json.JSONDecodeError, UnicodeError, ValueError):
                response = {"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "Parse error"}}
            if response is not None:
                stdout.write(json.dumps(response, ensure_ascii=False, allow_nan=False) + "\n")
                stdout.flush()

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", required=True, type=Path)
    parser.add_argument("--executable", type=Path)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--allow-write", action="store_true")
    args = parser.parse_args()
    try:
        server = Server(args.project, args.executable, allow_write=args.allow_write, output_dir=args.output_dir)
        server.serve(sys.stdin, sys.stdout)
    except (ValueError, OSError) as exc:
        parser.exit(1, f"CrySirror MCP: {exc}\n")
if __name__ == "__main__":
    main()
