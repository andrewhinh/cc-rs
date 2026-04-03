# cc-rs

A C compiler written in Rust.

![icon](./assets/icon.svg)

## Development

### System requirements

- [Linux](https://en.wikipedia.org/wiki/Linux) running [x86-64](https://en.wikipedia.org/wiki/X86-64) architecture. See instructions for running on AWS EC2 below.

### Installation

- [rustup](https://rustup.rs)
- [prek](https://prek.j178.dev)

```bash
prek install
```

EC2:

- [aws cli](https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html) and [auth setup](https://docs.aws.amazon.com/cli/latest/userguide/cli-chap-authentication.html)

```bash
IAM_USER=<iam-user> make aws-setup              # optionally specify IAM_USER to create access key, then create key pair
INSTANCE_TYPE=t3.medium ROOT_GB=50 make create  # optionally specify INSTANCE_TYPE, ROOT_GB, then create instance
make conn
SYNC=0 make conn CMD='cargo test'               # rerun remote commands without rsync
make conn-nosync                                # reconnect without rsync

# in the instance
cd ~/cc-rs && sudo apt install make && make ec2-setup && source "$HOME/.cargo/env"
```

### Commands

Run `make help` for the full list of commands.

For both Linux and EC2 instances:

```bash
cargo test
cargo run
```

## Roadmap

### Arithmetic and Basic Parsing

- [x] Compile an integer to an executable that exits with the given number
- [x] Add + and - operators
- [x] Add a tokenizer to allow space characters between tokens
- [x] Improve error message
- [x] Add *, / and ()
- [x] Add unary plus and minus
- [x] Add ==, !=, <= and >= operators
- [x] Read code from a file instead of argv[1]

### Statements, Control Flow, and Scopes

- [x] Accept multiple statements separated by semicolons
- [x] Support single-letter local variables
- [x] Support multi-letter local variables
- [x] Add "return" statement
- [x] Add { ... }
- [x] Add null statement
- [x] Add "if" statement
- [x] Add "for" statement
- [x] Add "while" statement
- [x] Add a representative node to each Node to improve error messages
- [x] Handle block scope
- [ ] Use hashmap for block-scope lookup
- [x] Allow for-loops to define local variables
- [x] Add goto and labeled statement
- [x] Resolve conflict between labels and typedefs
- [x] Add break statement
- [x] Add continue statement
- [x] Add switch-case
- [x] Add return that doesn't take any value
- [x] Add do ... while
- [ ] Make "main" to implicitly return 0
- [ ] [GNU] Support labels-as-values
- [ ] [GNU] Treat labels-as-values as compile-time constant

### Functions, Calls, and Calling Convention

- [x] Support zero-arity function calls
- [x] Support function call with up to 6 arguments
- [x] Support zero-arity function definition
- [x] Support function definition up to 6 parameters
- [x] Add function declaration
- [x] Handle return type conversion
- [x] Handle function argument type conversion
- [x] Support file-scope functions
- [x] Align stack frame to 16 byte boundaries
- [x] Handle a function returning bool, char or short
- [x] Allow to call a variadic function
- [x] Add va_start to support variadic functions
- [x] Check the number of function arguments
- [x] Allow to call a function that takes/returns flonums
- [x] Allow to define a function that takes/returns flonums
- [x] Implement default argument promotion for float
- [x] Support variadic function with floating-point parameters
- [x] Add function pointer
- [x] Decay a function to a pointer in the func param context
- [x] Add usual arithmetic conversion for function pointer
- [ ] Add va_arg()
- [ ] Support passed-on-stack arguments
- [ ] Support passed-on-stack parameters
- [ ] Allow struct parameter
- [ ] Allow struct argument
- [ ] Allow to call a fucntion returning a struct
- [ ] Allow to define a function returning a struct
- [ ] Allow variadic function to take more than 6 parameters
- [ ] Add va_copy()
- [ ] Dereferencing a function shouldn't do anything
- [ ] Handle inline functions as static functions
- [ ] Do not emit static inline functions if referenced by no one

### Type System, Declarations, and Semantics

- [x] Add keyword "int" and make variable definition mandatory
- [x] Add char type
- [x] Add comma operator
- [x] Change size of int from 8 to 4
- [x] Add long type
- [x] Add short type
- [x] Add nested type declarators
- [x] Add void type
- [x] Handle complex type declarations correctly
- [x] Add long long as an alias for long
- [x] Add typedef
- [x] Make sizeof to accept not only an expression but also a typename
- [x] Use 32 bit registers for char, short and int
- [x] Add type cast
- [x] Implement usual arithmetic conversion
- [x] Report an error on undefined/undeclared functions
- [x] Add _Bool type
- [x] Add enum
- [x] Add +=, -=, *= and /=
- [x] Add pre ++ and --
- [x] Add post ++ and --
- [x] Add hexadecimal, octal and binary number literals
- [x] Add ! operator
- [x] Add ~ operator
- [x] Add % and %=
- [x] Add &, |, ^, &=, |= and ^=
- [x] Add && and ||
- [x] Add <<, >>, <<= and >>=
- [x] Add ?: operator
- [x] Add constant expression
- [x] Add extern
- [x] Handle extern declarations in a block
- [ ] Detect function redefinition and conflicting redeclarations
- [x] Add _Alignof and _Alignas
- [x] [GNU] Allow a variable as an operand of _Alignof
- [x] Add static local variables
- [x] Add static global variables
- [x] Add signed keyword
- [x] Add unsigned integral types
- [x] Add U, L and LL suffixes
- [x] Use long or ulong instead of int for some expressions
- [x] When comparing two pointers, treat them as unsigned
- [x] Handle unsigned types in the constant expression
- [x] Ignore const, volatile, auto, register, restrict or _Noreturn.
- [x] Ignore "static" and "restrict" in array-dimensions
- [x] Allow to omit parameter name in function declaration
- [x] Add floating-point constant
- [x] Add "float" and "double" local variables and casts
- [x] Add flonum ==, !=, < and <=
- [x] Add flonum +, -, * and /
- [x] Handle flonum for if, while, do, !, ?:, || and &&
- [x] Add flonum constant expression
- [x] Add "long double" as an alias for "double"
- [ ] [GNU] Add ?: operator with omitted operand
- [ ] Add tentative definition
- [ ] Add thread-local variable
- [ ] Add alloca()
- [ ] Add sizeof() for VLA
- [ ] Add pointer arithmetic for VLA
- [ ] Support sizeof(typename) where typename is a VLA
- [ ] Do not define __STDC_NO_VLA__
- [ ] Emit size and type for symbols
- [ ] Add long double

### Pointers, Arrays, Structs, Unions, and Initializers

- [x] Add unary & and *
- [x] Make pointer arithmetic work
- [x] Add one dimensional arrays
- [x] Add arrays of arrays
- [x] Add [] operator
- [x] Add sizeof
- [x] Merge Function with Var
- [x] Add global variables
- [x] Add string literal
- [x] Add struct
- [x] Align struct members
- [x] Align local variables
- [x] Support struct tags
- [x] Add -> operator
- [x] Add union
- [x] Add struct assignment
- [x] Add a notion of an incomplete array type
- [x] Decay an array to a pointer in the func param context
- [x] Add a notion of an incomplete struct type
- [x] Support local variable initializers
- [x] Initialize excess array elements with zero
- [x] Skip excess initializer elements
- [x] Add string literal initializer
- [x] Allow to omit array length if an initializer is given
- [x] Handle struct initializers for local variables
- [x] Allow to initialize a struct with other struct
- [x] Handle union initializers for local variables
- [x] Add global initializer for scalar and string
- [x] Add struct initializer for global variable
- [x] Handle union initializers for global variable
- [x] Allow parentheses in initializers to be omitted
- [x] Allow extraneous braces for scalar initializer
- [x] Allow extraneous comma at the end of enum or initializer list
- [x] Emit uninitialized global data to .bss instead of .data
- [x] Add flexible array member
- [x] Allow to initialize struct flexible array member
- [x] Accept void as a parameter list
- [x] Align global variables
- [x] Add compound literals
- [ ] Add bitfield
- [ ] Support global struct bitfield initializer
- [ ] Handle op=-style assignments to bitfields
- [ ] Handle zero-width bitfield member
- [ ] Do not allow to obtain an address of a bitfield
- [ ] Make an array of at least 16 bytes long to have alignment of at least 16 bytes
- [ ] Add anonymous struct and union
- [ ] Add array designated initializer
- [ ] Allow array designators to initialize incomplete arrays
- [ ] [GNU] Allow to omit "=" in designated initializers
- [ ] Add struct designated initializer
- [ ] Add union designated initializer
- [ ] Handle struct designator for anonymous struct member
- [ ] Add offsetof
- [ ] [GNU] Support array range designator
- [ ] Add __attribute__((packed))
- [ ] Add __attribute__((aligned(N)) for struct declaration
- [ ] Make struct member access to work with = and ?:

### Lexer, Literals, Unicode, and Tokens

- [x] Add \a, \b, \t, \n \v, \f, \r and \e
- [x] Add \<octal-sequence>
- [x] Add \x<hexadecimal-sequence>
- [x] Add line and block comments
- [x] Add character literal
- [ ] Concatenate adjacent string literals
- [ ] Recognize wide character literal
- [ ] Tokenize numeric tokens as pp-numbers
- [x] Precompute line number for each token
- [ ] Use hashmap for keyword lookup
- [ ] Canonicalize newline character
- [ ] Add \u and \U escape sequences
- [ ] Accept multibyte character as wide character literal
- [ ] Add UTF-16 character literal
- [ ] Add UTF-32 character literal
- [ ] Add UTF-8 string literal
- [ ] Add UTF-16 string literal
- [ ] Add UTF-32 string literal
- [ ] Add wide string literal
- [ ] Add UTF-16 string literal initializer
- [ ] Add UTF-32 string literal initializer
- [ ] Define __STDC_UTF_{16,32}__ macros
- [ ] Allow multibyte UTF-8 character in identifier
- [ ] [GNU] Accept $ as an identifier character
- [ ] Allow to concatenate regular string literals with L/u/U string literals
- [ ] Skip UTF-8 BOM markers
- [ ] Improve error message for multibyte characters

### Preprocessor, Includes, and Macro System

- [x] Add a do-nothing preprocessor
- [x] Add the null directive
- [x] Add #include "..."
- [x] Skip extra tokens after #include "..."
- [x] Add -E option
- [x] Add #if and #endif
- [x] Skip nested #if in a skipped #if-clause
- [x] Add #else
- [x] Add #elif
- [x] Add objlike #define
- [x] Add #undef
- [x] Expand macros in the #if and #elif argument context
- [x] Do not expand a token more than once for the same objlike macro
- [x] Add #ifdef and #ifndef
- [x] Add zero-arity funclike #define
- [x] Add multi-arity funclike #define
- [x] Allow empty macro arguments
- [x] Allow parenthesized expressions as macro arguments
- [x] Do not expand a token more than once for the same funclike macro
- [x] Add macro stringizing operator (#)
- [x] Add macro token-pasting operator (##)
- [x] Use C compiler's preprocessor for all tests
- [x] Add defined() macro operator
- [x] Replace remaining identifiers with 0 in macro constexpr
- [ ] Preserve newline and space during macro expansion
- [ ] Support line continuation
- [ ] Add #include <...>
- [ ] Add -I option
- [ ] Add default include paths
- [ ] Add #error
- [ ] Add predefine macros such as __STDC__
- [ ] Add __FILE__ and __LINE__
- [ ] Add __VA_ARGS__
- [ ] Add __func__
- [ ] [GNU] Add __FUNCTION__
- [ ] Add __DATE__ and __TIME__ macros
- [ ] [GNU] Add __COUNTER__ macro
- [ ] Add #line
- [ ] [GNU] Add line marker directive
- [ ] [GNU] Add __TIMESTAMP__ macro
- [ ] [GNU] Add __BASE_FILE__ macro
- [ ] Add __VA_OPT__
- [ ] [GNU] Handle ,##__VA_ARG__
- [ ] Ignore #pragma
- [ ] [GNU] Support GCC-style variadic macro
- [ ] Add -idirafter option
- [ ] Add -include option
- [ ] Add string hashmap
- [ ] Use hashmap for macro name lookup
- [ ] Cache file search results
- [ ] Add include guard optimization
- [ ] [GNU] Add "#pragma once"
- [ ] [GNU] Add #include_next

### Driver, Toolchain Integration, and Build Flags

- [x] Add -o and --help options
- [ ] Rewrite tests in shell script in C
- [x] Emit .file and .loc assembler directives
- [x] Add stage2 build
- [ ] Self-host: including preprocessor, the C compiler can compile itself
- [x] Split cc1 from compiler driver
- [x] Run "as" command unless -S is given
- [x] Accept multiple input files
- [x] Run "ld" unless -c is given
- [ ] Add stdarg.h, stdbool.h, stddef.h, stdalign.h and float.h
- [ ] Add -D option
- [ ] Add -U option
- [ ] Write to an in-memory buffer before writing to an actual output file
- [ ] Ignore -O, -W and -g and other flags
- [ ] Turn on -Wall compiler flag and fix compiler warnings
- [ ] Add -fcommon and -fno-common flags
- [ ] Add -x option
- [ ] Make -E to imply -xc
- [ ] Add -l option
- [ ] Add -s option
- [ ] Recognize .a and .so files
- [ ] Add -M option
- [ ] Add -MF option
- [ ] Add -MP option
- [ ] Add -MT option
- [ ] Add -MD option
- [ ] Add -MQ option
- [ ] Add -MMD option
- [ ] Add -fpic and -fPIC options
- [ ] Add -static option
- [ ] Add -shared option
- [ ] Add -L option
- [ ] Add -Wl, option
- [ ] Add -Xlinker option

### GNU/C11 Extensions, Atomics, and Advanced Features

- [x] [GNU] Add statement expression
- [ ] Add typeof
- [ ] [GNU] Add __builtin_types_compatible_p
- [ ] Add _Generic
- [ ] [GNU] Allow sizeof(<function type>)
- [ ] Add basic "asm" statement
- [ ] [GNU] Support case ranges
- [ ] Add atomic_compare_exchange
- [ ] Add atomic_exchange
- [ ] Add _Atomic and atomic ++, -- and op= operators
- [ ] Complete stdatomic.h

## Credit

- [An Incremental Approach to Compiler Construction](http://scheme2006.cs.uchicago.edu/11-ghuloum.pdf)
- [chibicc](https://github.com/rui314/chibicc)
- [Writing a Compiler in Rust](https://thume.ca/2019/04/18/writing-a-compiler-in-rust/)
