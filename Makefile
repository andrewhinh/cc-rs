.PHONY: help conn conn-nosync create delete list reboot aws-setup ec2-setup start stop stage2 test-stage2 test-all clean FORCE

ARG := $(word 2,$(MAKECMDGOALS))
CMD ?=
HOST ?=
SYNC ?= 1
SRCS := $(wildcard src/cpp/*.c)
TEST_SRCS := $(wildcard src/cpp/test/*.c)
RUST_SRCS := Cargo.toml Cargo.lock $(wildcard src/*.rs)
TEST_TIMEOUT ?= 120
CC_RS := ./target/release/cc-rs
STAGE2_CC_RS := ./target/release/cc-rs
TESTS := $(TEST_SRCS:.c=.exe)
CC_RS_S := $(CC_RS) -S
STAGE2_CC_RS_S := $(STAGE2_CC_RS) -S
CPP_INCLUDES := -Isrc/cpp/include -Isrc/cpp/test

help:
	@echo "conn        sync and connect to instance [instance-id] [CMD=...] [SYNC=0]"
	@echo "conn-nosync connect to instance without rsync [instance-id] [CMD=...]"
	@echo "create      create instance"
	@echo "delete      delete instance <instance-id>"
	@echo "list        list instances"
	@echo "reboot      reboot instance <instance-id>"
	@echo "aws-setup   setup aws account"
	@echo "ec2-setup   setup connected ec2 instance"
	@echo "start       start instance <instance-id>"
	@echo "stop        stop instance <instance-id>"

conn:
ifdef CMD
	SYNC=$(SYNC) bash scripts/conn.sh $(ARG) --cmd '$(CMD)'
else
	SYNC=$(SYNC) bash scripts/conn.sh $(ARG)
endif

conn-nosync:
ifdef CMD
	bash scripts/conn.sh $(ARG) --no-sync --cmd '$(CMD)'
else
	bash scripts/conn.sh $(ARG) --no-sync
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

stage2/cpp: $(SRCS:src/cpp/%.c=stage2/%.s)
	@mkdir -p stage2
	gcc -no-pie -o $@ $^

stage2/%.s: src/cpp/%.c target/release/cc-rs
	@mkdir -p stage2
	$(STAGE2_CC_RS_S) $(CPP_INCLUDES) -o $@ $<

target/debug/cc-rs: $(RUST_SRCS)
	cargo build

target/release/cc-rs: $(RUST_SRCS)
	cargo build --release

test-stage2: stage2/cpp
	@mkdir -p stage2/test
	@for i in $(TEST_SRCS); do \
		name=$$(basename $${i%.c}); \
		echo $$i; \
		timeout $(TEST_TIMEOUT)s ./stage2/cpp -S $(CPP_INCLUDES) -o stage2/test/$$name.s $$i || exit 1; \
		timeout $(TEST_TIMEOUT)s gcc -no-pie -pthread -o stage2/test/$$name.exe stage2/test/$$name.s -xc src/cpp/test/common || exit 1; \
		timeout $(TEST_TIMEOUT)s ./stage2/test/$$name.exe || exit 1; \
		echo; \
	done
	timeout $(TEST_TIMEOUT)s bash src/cpp/test/driver.sh ./stage2/cpp

test-all: test test-stage2

test: target/release/cc-rs
	@echo "Running basic tests..."
	@for i in $(TEST_SRCS); do \
		name=$$(basename $${i%.c}); \
		echo $$i; \
		timeout $(TEST_TIMEOUT)s $(CC_RS) -S $(CPP_INCLUDES) -o /tmp/$$name.s $$i; \
		timeout $(TEST_TIMEOUT)s gcc -no-pie -pthread -o /tmp/$$name /tmp/$$name.s -xc src/cpp/test/common; \
		timeout $(TEST_TIMEOUT)s /tmp/$$name || exit 1; \
		echo; \
	done
	timeout $(TEST_TIMEOUT)s bash src/cpp/test/driver.sh $(CC_RS)

clean:
	rm -rf stage2
