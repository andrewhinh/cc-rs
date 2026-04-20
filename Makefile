.PHONY: help conn conn-nosync create delete list reboot aws-setup ec2-setup start stop stage2 test-stage2 test-all clean FORCE

ARG := $(word 2,$(MAKECMDGOALS))
HOST ?=
SYNC ?= 1

help:
	@echo "conn        sync and connect to instance <instance-id> [SYNC=0]"
	@echo "conn-nosync connect to instance without rsync <instance-id>"
	@echo "create      create instance"
	@echo "delete      delete instance <instance-id>"
	@echo "list        list instances"
	@echo "reboot      reboot instance <instance-id>"
	@echo "aws-setup   setup aws account"
	@echo "ec2-setup   setup connected ec2 instance"
	@echo "start       start instance <instance-id>"
	@echo "stop        stop instance <instance-id>"

conn:
	SYNC=$(SYNC) bash scripts/conn.sh $(ARG)

conn-nosync:
	bash scripts/conn.sh $(ARG) --no-sync

create:
	bash scripts/create.sh

delete:
	bash scripts/delete.sh $(ARG)

list:
	bash scripts/list.sh

reboot:
	bash scripts/reboot.sh $(ARG)

aws-setup:
	bash scripts/aws-setup.sh

ec2-setup:
	bash scripts/ec2-setup.sh

start:
	bash scripts/start.sh $(ARG)

stop:
	bash scripts/stop.sh $(ARG)

$(ARG):
	@:
