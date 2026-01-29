// pty_launcher: Creates a PTY via forkpty(), execs the given command on the
// slave side, and holds the master fd open. When this process is killed, the
// master fd closes, which causes SIGHUP to be sent to the child and the slave
// side to report POLLHUP/EIO -- the same behavior as closing a real terminal
// window or tab.
//
// Usage: pty_launcher <command> [args...]
//
// This is used by test-pty-close.sh because macOS `script -q /dev/null` does
// NOT properly close the PTY master when killed (the child becomes the session
// leader and keeps the PTY alive).
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>
#ifdef __linux__
#include <pty.h>
#else
#include <util.h>
#endif

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s command [args...]\n", argv[0]);
        return 1;
    }

    int master;
    pid_t pid = forkpty(&master, NULL, NULL, NULL);
    if (pid < 0) {
        perror("forkpty");
        return 1;
    }

    if (pid == 0) {
        // Child: exec the command on the slave side of the PTY
        execvp(argv[1], &argv[1]);
        perror("exec");
        _exit(127);
    }

    // Parent: holds the master fd. Drain output until killed.
    // Write our child PID to stderr so the test script can find it.
    fprintf(stderr, "pty_launcher: master_pid=%d child_pid=%d\n", getpid(), pid);

    char buf[4096];
    while (1) {
        ssize_t n = read(master, buf, sizeof(buf));
        if (n <= 0) break;
    }

    int status;
    waitpid(pid, &status, 0);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}
