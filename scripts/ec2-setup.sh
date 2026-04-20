#!/usr/bin/env bash
set -euo pipefail

sudo apt-get update
sudo apt-get install -y git docker.io build-essential pkg-config libffi-dev libssl-dev wget curl

if getent group docker >/dev/null; then
  sudo usermod -aG docker "$USER"
fi

if [[ ! -f "$HOME/.cargo/env" ]]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi

cd ~/cc-rs
echo "ec2 setup complete"
