#!/usr/bin/env bash
set -euo pipefail

install_dir="${AAQE_BIN_DIR:-${HOME}/.local/bin}"
version="${AAQE_VERSION:-latest}"
release_base="${AAQE_RELEASE_BASE_URL:-https://github.com/qevolg/ai-agent-qe/releases}"
work_dir=""
install_file=""

cleanup() {
    if [[ -n "$work_dir" ]]; then rm -rf "$work_dir"; fi
    if [[ -n "$install_file" ]]; then rm -f "$install_file"; fi
}
trap cleanup EXIT

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version) version="${2:?--version requires a version}"; shift 2 ;;
        --bin-dir|-b) install_dir="${2:?--bin-dir requires a directory}"; shift 2 ;;
        --help|-h)
            echo 'Usage: bash install.sh [--version VERSION] [--bin-dir DIRECTORY]'
            echo 'Installs AAQE for Linux x86_64; default: latest release to ~/.local/bin.'
            exit 0 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
    echo 'AAQE binary installation supports Linux x86_64 only.' >&2
    exit 1
fi
for command_name in curl tar sha256sum mktemp install mv sync; do
    command -v "$command_name" >/dev/null || { echo "Missing command: $command_name" >&2; exit 1; }
done

if [[ "$version" == latest ]]; then
    download_url="${release_base%/}/latest/download"
else
    [[ "$version" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$ ]] || { echo 'Invalid version.' >&2; exit 1; }
    download_url="${release_base%/}/download/v${version#v}"
fi

work_dir="$(mktemp -d)"
archive_name='aaqe-x86_64-unknown-linux-musl.tar.gz'
curl --fail --silent --show-error --location --retry 3 --connect-timeout 15 --max-time 300 \
    "${download_url}/${archive_name}" -o "${work_dir}/${archive_name}"
curl --fail --silent --show-error --location --retry 3 --connect-timeout 15 --max-time 60 \
    "${download_url}/${archive_name}.sha256" -o "${work_dir}/checksum"
read -r expected_checksum _ < "${work_dir}/checksum" || [[ -n "$expected_checksum" ]]
[[ "$expected_checksum" =~ ^[a-fA-F0-9]{64}$ ]] || { echo 'Invalid SHA-256 checksum.' >&2; exit 1; }
actual_checksum="$(sha256sum "${work_dir}/${archive_name}")"
[[ "${actual_checksum%% *}" == "${expected_checksum,,}" ]] || { echo 'SHA-256 verification failed.' >&2; exit 1; }
tar -xzf "${work_dir}/${archive_name}" -C "$work_dir" aaqe
[[ -f "${work_dir}/aaqe" && ! -L "${work_dir}/aaqe" ]] || { echo 'Archive does not contain a regular aaqe binary.' >&2; exit 1; }
"${work_dir}/aaqe" --version
mkdir -p "$install_dir"
install_file="$(mktemp "${install_dir}/.aaqe.XXXXXX")"
install -m 755 "${work_dir}/aaqe" "$install_file"
# Flush the complete replacement before switching the entry atomically.
sync -f "$install_file"
mv -fT -- "$install_file" "${install_dir}/aaqe"
sync -f "$install_dir"
install_file=""
echo "Installed AAQE to ${install_dir}/aaqe"
case ":${PATH}:" in
    *":${install_dir}:"*) ;;
    *) printf 'Add this directory to PATH: export PATH="%s:$PATH"\n' "$install_dir" ;;
esac
echo 'Run aaqe doctor to check Git, tmux, Codex and Docker.'
