"""仅迁移已确认的单条 CelesVault 子树历史，不改父仓库索引或工作树。"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PARENT = ROOT.parent
BRANCH = "Wemsur/feat/app-shell"


def git(directory, *args, data=None):
    return subprocess.run(["git", "-C", str(directory), *args], input=data,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    if (ROOT / ".git").exists():
        raise SystemExit("独立 .git 已存在，拒绝覆盖或重复迁移。")
    parent_top = Path(git(PARENT, "rev-parse", "--show-toplevel").decode().strip()).resolve()
    if parent_top != PARENT.resolve():
        raise SystemExit("父仓库位置发生变化，停止迁移。")
    commits = git(PARENT, "rev-list", "HEAD", "--", "CelesVault").decode().splitlines()
    if len(commits) != 1:
        raise SystemExit("路径历史不再只有一条，必须重新设计迁移。")
    old = commits[0]
    raw = git(PARENT, "cat-file", "commit", old)
    if b"\nparent " in raw.split(b"\n\n", 1)[0]:
        raise SystemExit("基线不再是根提交，停止迁移。")
    if git(PARENT, "ls-tree", "--name-only", old).decode().splitlines() != ["CelesVault"]:
        raise SystemExit("根提交还含其他项目，停止迁移。")
    subtree = git(PARENT, "rev-parse", f"{old}:CelesVault").decode().strip()
    parent_status = git(PARENT, "status", "--porcelain=v1", "-z", "--untracked-files=normal")
    parent_staged = git(PARENT, "ls-files", "--stage", "-z")
    parent_head = git(PARENT, "rev-parse", "HEAD")
    parent_branch = git(PARENT, "symbolic-ref", "HEAD")
    index_path = PARENT / git(PARENT, "rev-parse", "--git-path", "index").decode().strip()
    index_bytes = index_path.read_bytes()
    paths = git(PARENT, "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "CelesVault")
    paths = sorted(set(p.decode("utf-8") for p in paths.split(b"\0") if p))
    temp = Path(tempfile.mkdtemp(prefix="CelesVault-history-"))
    bare = temp / "repository.git"
    subprocess.run(["git", "init", "--bare", "--initial-branch=main", str(bare)], check=True)
    backup = bare / "migration-backup"
    backup.mkdir()
    (backup / "parent-status.bin").write_bytes(parent_status)
    (backup / "parent-staged.bin").write_bytes(parent_staged)
    (backup / "parent-index.bin").write_bytes(index_bytes)
    manifest = []
    with zipfile.ZipFile(backup / "working-source.zip", "x", zipfile.ZIP_DEFLATED) as archive:
        for path in paths:
            source = PARENT / path
            if not source.is_file():
                raise RuntimeError(f"快照文件不可读：{path}")
            relative = source.relative_to(ROOT).as_posix()
            contents = source.read_bytes()
            archive.writestr(relative, contents)
            manifest.append({"path": relative, "size": len(contents), "sha256": sha(contents)})
    (backup / "working-manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    packed = git(PARENT, "pack-objects", "--stdout", "--revs", data=(subtree + "\n").encode())
    git(bare, "index-pack", "--stdin", data=packed)
    new_raw = b"tree " + subtree.encode() + b"\n" + raw.split(b"\n", 1)[1]
    new = git(bare, "hash-object", "-t", "commit", "-w", "--stdin", data=new_raw).decode().strip()
    git(bare, "update-ref", "refs/heads/main", new)
    git(bare, "update-ref", "refs/heads/" + BRANCH, new)
    git(bare, "symbolic-ref", "HEAD", "refs/heads/" + BRANCH)
    git(bare, "config", "core.bare", "false")
    git(bare, "config", "core.logallrefupdates", "true")
    for key in ("user.name", "user.email"):
        git(bare, "config", key, git(PARENT, "config", "--get", key).decode().strip())
    if git(bare, "cat-file", "commit", new) != new_raw:
        raise RuntimeError("提交元数据未原样保留。")
    if (bare / "objects" / "info" / "alternates").exists():
        raise RuntimeError("对象库不能依赖父仓库。")
    if git(PARENT, "ls-files", "--stage", "-z") != parent_staged or index_path.read_bytes() != index_bytes:
        raise RuntimeError("父仓库索引变化，停止安装独立仓库。")
    os.rename(bare, ROOT / ".git")
    git(ROOT, "read-tree", "HEAD")
    if git(PARENT, "rev-parse", "HEAD") != parent_head or git(PARENT, "symbolic-ref", "HEAD") != parent_branch:
        raise RuntimeError("父仓库引用变化。")
    if git(PARENT, "ls-files", "--stage", "-z") != parent_staged or index_path.read_bytes() != index_bytes:
        raise RuntimeError("父仓库索引变化。")
    for entry in manifest:
        if sha((ROOT / entry["path"]).read_bytes()) != entry["sha256"]:
            raise RuntimeError("工作树快照校验失败：" + entry["path"])
    result = {"old_commit": old, "new_commit": new, "subtree": subtree, "branch": BRANCH,
              "parent_branch": parent_branch.decode().strip(), "parent_index_sha256": sha(index_bytes),
              "parent_staged_sha256": sha(parent_staged), "source_file_count": len(manifest),
              "parent_index_unchanged": True, "source_unchanged": True, "standalone_objects": True,
              "backup": ".git/migration-backup", "parent_tracking_retained": True}
    history = ROOT / "docs" / "history"
    history.mkdir(parents=True, exist_ok=True)
    (history / "migration-2026-11-14.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temp.rmdir()
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
