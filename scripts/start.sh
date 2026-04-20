#!/usr/bin/env bash
set -euo pipefail

INSTANCE_ID=""
SSH_READY_TIMEOUT="${SSH_READY_TIMEOUT:-120}"

if [[ $# -gt 1 ]]; then
  echo "Usage: $(basename "$0") [instance-id]"
  exit 1
fi

if [[ $# -eq 1 ]]; then
  if [[ "$1" == "-h" || "$1" == "--help" ]]; then
    echo "Usage: $(basename "$0") [instance-id]"
    exit 0
  fi
  INSTANCE_ID="$1"
fi

REGION=$(aws configure get region)

wait_for_ssh() {
  local instance_id="$1"
  local public_ip="$2"
  local known_hosts="${CC_RS_KNOWN_HOSTS:-$HOME/.ssh/known_hosts_cc_rs}"
  local deadline=$((SECONDS + SSH_READY_TIMEOUT))
  local ssh_opts=(
    -i cc-rs.pem
    -o BatchMode=yes
    -o CheckHostIP=no
    -o ConnectTimeout=5
    -o StrictHostKeyChecking=accept-new
    -o UserKnownHostsFile="$known_hosts"
    -o HostKeyAlias="cc-rs-$instance_id"
  )

  if [[ -z "${public_ip}" || "${public_ip}" == "None" ]]; then
    echo "No public IP for ${instance_id}"
    return 1
  fi

  mkdir -p "$HOME/.ssh"
  until ssh "${ssh_opts[@]}" ubuntu@"$public_ip" true 2>/dev/null; do
    if (( SECONDS >= deadline )); then
      echo "Timed out waiting for SSH on ${instance_id} (${public_ip})"
      return 1
    fi
    sleep 5
  done
}

if [[ -n "${INSTANCE_ID}" ]]; then
  INSTANCE_IDS="${INSTANCE_ID}"
else
  INSTANCE_IDS="$(aws ec2 describe-instances --region "$REGION" \
    --filters Name=tag:Name,Values=cc-rs Name=instance-state-name,Values=stopped \
    --query 'Reservations[].Instances[].InstanceId' --output text)"
fi

if [[ -z "${INSTANCE_IDS}" || "${INSTANCE_IDS}" == "None" ]]; then
  echo "No stopped cc-rs instances found"
  exit 0
fi

aws ec2 start-instances --region "$REGION" --instance-ids ${INSTANCE_IDS}
aws ec2 wait --region "$REGION" instance-running --instance-ids ${INSTANCE_IDS}

INSTANCE_ROWS="$(aws ec2 describe-instances --region "$REGION" --instance-ids ${INSTANCE_IDS} \
  --query 'Reservations[].Instances[].[InstanceId,PublicIpAddress]' --output text)"

echo "Started instances: ${INSTANCE_IDS}"
echo "Public IPs:"
echo "${INSTANCE_ROWS}"

while read -r started_instance_id public_ip; do
  wait_for_ssh "$started_instance_id" "$public_ip"
done <<< "${INSTANCE_ROWS}"
echo "SSH ready"
