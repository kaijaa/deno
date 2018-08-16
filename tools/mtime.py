#!/usr/bin/env python
import os
import subprocess

root_path = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
os.chdir(root_path)


def git_mtime(filename):
    output = subprocess.check_output(
        ["git", "log", "--pretty=%at", "-1", "--", filename]).strip()
    if output:
        #print output, filename
        ctime = int(output)
        print ctime, filename
        os.utime(filename, (ctime, ctime))


for root, dirs, files in os.walk(root_path):
    for f in files:
        filename = os.path.join(root, f)
        git_mtime(filename)
