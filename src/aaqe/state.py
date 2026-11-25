import json
import os
import re
import uuid
from datetime import datetime, timezone
from pathlib import Path

from .errors import AAQEError


def now():
    return datetime.now(timezone.utc).isoformat()


def write_json(path, document):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    temporary.write_text(
        json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    os.replace(temporary, path)


def read_json(path):
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise AAQEError(f"Cannot read artifact {path}: {error}") from error


class Store:
    def __init__(self, root):
        self.root = Path(root).expanduser().resolve()

    def path(self, run_id):
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]{0,99}", run_id):
            raise AAQEError("Invalid run ID")
        path = self.root / run_id
        if path.is_symlink():
            raise AAQEError("Run directories cannot be symbolic links")
        return path

    def create(self, prompt, config, caller):
        run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex[:8]
        directory = self.path(run_id)
        directory.mkdir(parents=True, exist_ok=False)
        if os.name != "nt":
            directory.chmod(0o700)
        state = {
            "id": run_id,
            "created_at": now(),
            "prompt": prompt,
            "caller": str(Path(caller).resolve()),
            "config": config.to_dict(),
            "status": "initializing",
            "stage": 0,
            "stages": [],
            "error": None,
            "workspace": None,
            "environment": None,
        }
        self.save(state)
        return state

    def load(self, run_id):
        return read_json(self.path(run_id) / "run.json")

    def save(self, state):
        write_json(self.path(state["id"]) / "run.json", state)
