# AWS Relay Deployment

This folder contains numbered scripts for deploying `grid-relayd` to an AWS EC2 instance.

## Stages

Run scripts in this order:

1. `01-check-prereqs.sh`
2. `02-create-security-group.sh`
3. `03-launch-instance.sh`
4. `04-build-relay.sh`
5. `05-deploy-service.sh`
6. `06-verify.sh`

## Quick Start

Auth-only path (auto-discovery defaults):

```bash
cd deploy/aws/relay
bash 01-check-prereqs.sh
bash 02-create-security-group.sh
bash 03-launch-instance.sh
bash 04-build-relay.sh
bash 05-deploy-service.sh
bash 06-verify.sh
```

Optional custom path with explicit `.env` overrides:

```bash
cd deploy/aws/relay
cp 00-env.example .env
# edit only the values you want to override

bash 01-check-prereqs.sh
bash 02-create-security-group.sh
bash 03-launch-instance.sh
bash 04-build-relay.sh
bash 05-deploy-service.sh
bash 06-verify.sh
```

## Notes

- Scripts use AWS CLI and SSH.
- State is written to `.state` in this directory.
- `EC2_VPC_ID`, `EC2_SUBNET_ID`, and `EC2_AMI_ID` auto-resolve when omitted.
- If key settings are omitted, stage 03 creates a temporary EC2 key pair and stores path in `.state`.
- `04-build-relay.sh` clones from `https://github.com/cypher-asi/the-grid` on the Ubuntu host and builds there.
- `05-deploy-service.sh` installs a `systemd` service named `grid-relayd` using the host-built binary.
- Make scripts executable if needed: `chmod +x *.sh`.
