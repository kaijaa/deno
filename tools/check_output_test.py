#!/usr/bin/env python
# Given a deno executable, this script execute several integration tests
# with it. The tests are stored in //tests/ and each script has a corresponding
# .out file which specifies what the stdout should be.
#
# Usage: check_output_test.py [path to deno executable]
import os
import sys
import subprocess
from util import pattern_match

root_path = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
tests_path = os.path.join(root_path, "tests")

# Not thread safe.
child_processes = []


def check_output_wait():
    for (cmd, script, out_filename, p) in child_processes:
        p.wait()
        out_abs = os.path.join(tests_path, out_filename)
        with open(out_abs, 'r') as f:
            expected_out = f.read()
        should_succeed = "error" not in script
        print " ".join(cmd)
        out, _ = p.communicate()
        err = (p.returncode != 0)
        actual_out = out
        if should_succeed and err:
            print "Expected success but got error. Output:"
            print actual_out
            sys.exit(1)

        if should_succeed == False and not err:
            print "Expected an error but succeeded. Output:"
            print actual_out
            sys.exit(1)

        if pattern_match(expected_out, actual_out) != True:
            print "Expected output does not match actual."
            print "Expected: " + expected_out
            print "Actual:   " + actual_out
            sys.exit(1)


def check_output_test(deno_exe_filename):
    assert os.path.isfile(deno_exe_filename)
    outs = sorted([
        filename for filename in os.listdir(tests_path)
        if filename.endswith(".out")
    ])
    assert len(outs) > 1
    tests = [(os.path.splitext(filename)[0], filename) for filename in outs]

    for (script, out_filename) in tests:
        script_abs = os.path.join(tests_path, script)
        cmd = [deno_exe_filename, script_abs, "--reload"]
        p = subprocess.Popen(
            cmd,
            universal_newlines=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE)
        child_processes.append((cmd, script, out_filename, p))


def main(argv):
    check_output_test(argv[1])
    check_output_wait()


if __name__ == '__main__':
    sys.exit(main(sys.argv))
