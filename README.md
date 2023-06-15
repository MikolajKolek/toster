# Toster
[![Crates.io](https://img.shields.io/crates/l/toster)](https://github.com/MikolajKolek/toster/blob/master/LICENSE)
[![Crates.io](https://img.shields.io/crates/d/toster)](https://crates.io/crates/toster)
[![Crates.io](https://img.shields.io/crates/v/toster)](https://crates.io/crates/toster)

A simple-as-toast tester for C++ solutions to competitive programming exercises

# Usage

```
Usage: toster [OPTIONS] <FILENAME>

Arguments:
  <FILENAME>  The name of the file containing the source code or the executable you want to test

Options:
  -i, --in <IN>
          Input directory [default: in]
      --in-ext <IN_EXT>
          Input file extension [default: .in]
  -o, --out <OUT>
          Output directory [default: out]
      --out-ext <OUT_EXT>
          Output file extension [default: .out]
      --io <IO>
          The input and output directory (sets both -i and -o at once)
  -t, --timeout <TIMEOUT>
          The number of seconds after which a test or generation times out if the program does not return [default: 5]
      --compile-timeout <COMPILE_TIMEOUT>
          The number of seconds after which compilation times out if it doesn't finish [default: 10]
  -c, --compile-command <COMPILE_COMMAND>
          The command used to compile the file. <IN> gets replaced with the path to the source code file, <OUT> is the executable output location [default: "g++ -std=c++17 -O3 -static <IN> -o <OUT>"]
  -g, --generate
          Makes the tester generate output files in the output directory instead of comparing the program's output with the files in the output directory
  -h, --help
          Print help information
  -V, --version
          Print version information
```

# License
Toster is licensed under the [MIT Licence](https://github.com/MikolajKolek/toster/blob/master/LICENSE)

# Dependencies
This project uses [sio2jail](https://github.com/sio2project/sio2jail), a project available under the MIT licence