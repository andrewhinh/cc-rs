.PHONY: help conn create delete list reboot aws-setup ec2-setup start stop

ARG := $(word 2,$(MAKECMDGOALS))
CMD ?=
HOST ?=

help:
	@echo "conn       connect to instance [instance-id] [CMD=...]"
	@echo "create     create instance"
	@echo "delete     delete instance <instance-id>"
	@echo "list       list instances"
	@echo "reboot     reboot instance <instance-id>"
	@echo "aws-setup  setup aws account"
	@echo "ec2-setup  setup connected ec2 instance"
	@echo "start      start instance <instance-id>"
	@echo "stop       stop instance <instance-id>"

conn:
ifdef CMD
	bash scripts/conn.sh $(ARG) --cmd '$(CMD)'
else
	bash scripts/conn.sh $(ARG)
endif

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
