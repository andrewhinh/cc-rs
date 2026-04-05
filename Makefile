.PHONY: help conn create delete list reboot aws-setup ec2-setup start stop stage2 test-stage2 test-all

ARG := $(word 2,$(MAKECMDGOALS))
CMD ?=
HOST ?=
SRCS := $(wildcard chibicc/*.c)
TEST_SRCS := $(wildcard chibicc/test/*.c)
TESTS := $(TEST_SRCS:.c=.exe)

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

# Stage 2 build

stage2/chibicc: $(SRCS:chibicc/%.c=stage2/%.s)
	@mkdir -p stage2
	gcc -no-pie -o $@ $^

stage2/%.s: chibicc/%.c target/debug/cc-rs
	@mkdir -p stage2
	./target/debug/cc-rs -o $@ $<

target/debug/cc-rs:
	cargo build

test-stage2: stage2/chibicc
	@for i in $(TEST_SRCS); do \
		echo $$i; \
		./stage2/chibicc -Ichibicc/include -Ichibicc/test -o stage2/test/$$(basename $${i%.c}).s $$i; \
		gcc -no-pie -o stage2/test/$$(basename $${i%.c}).exe stage2/test/$$(basename $${i%.c}).s -xc chibicc/test/common; \
		./stage2/test/$$(basename $${i%.c}).exe || exit 1; \
		echo; \
	done
	bash chibicc/test/driver.sh ./stage2/chibicc

test-all: test test-stage2

test: target/debug/cc-rs
	@echo "Running basic tests..."
	@for i in $(TEST_SRCS); do \
		echo $$i; \
		./target/debug/cc-rs -Ichibicc/include -Ichibicc/test -o /tmp/$$(basename $${i%.c}).s $$i; \
		gcc -no-pie -o /tmp/$$(basename $${i%.c}) /tmp/$$(basename $${i%.c}).s -xc chibicc/test/common; \
		/tmp/$$(basename $${i%.c}) || exit 1; \
		echo; \
	done

clean:
	rm -rf stage2
