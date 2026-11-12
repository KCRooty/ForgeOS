/* bellows.c — Bellows v0: el primer shell de ForgeOS que corre en ring 3.
 *
 * Todavía un boceto del shell de docs/SHELL.md: lee una línea, la parte
 * en palabras y ejecuta el programa con fork() + execv() + waitpid().
 * Sin tuberías, redirecciones, comillas ni variables.
 *
 *   forge> run /bin/bellows
 *   bellows> hello
 *   bellows> libctest uno dos
 *   bellows> /bin/crash div0
 *   bellows> exit
 *
 * Un nombre sin '/' se busca en /bin. Órdenes internas: help, exit [n],
 * pid. Ctrl-D (fin de fichero) también sale.
 */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define MAX_LINE 512
#define MAX_WORDS 32

/* Parte `line` en palabras separadas por blancos (la modifica). */
static int split(char *line, char **words) {
    int count = 0;
    char *p = line;
    while (*p) {
        while (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')
            *p++ = '\0';
        if (!*p)
            break;
        if (count == MAX_WORDS - 1)
            return -1;
        words[count++] = p;
        while (*p && *p != ' ' && *p != '\t' && *p != '\n' && *p != '\r')
            p++;
    }
    words[count] = NULL;
    return count;
}

static const char *signal_name(int sig) {
    switch (sig) {
    case SIGINT:  return "SIGINT";
    case SIGILL:  return "SIGILL";
    case SIGABRT: return "SIGABRT";
    case SIGFPE:  return "SIGFPE";
    case SIGKILL: return "SIGKILL";
    case SIGSEGV: return "SIGSEGV";
    case SIGTERM: return "SIGTERM";
    default:      return "senal";
    }
}

/* Ejecuta un programa y espera a que termine. Devuelve su código al
 * estilo de un shell: el de exit(), o 128 + señal. */
static int run(char **words) {
    char path[MAX_LINE + 8];
    if (strchr(words[0], '/'))
        snprintf(path, sizeof path, "%s", words[0]);
    else
        snprintf(path, sizeof path, "/bin/%s", words[0]);

    fflush(stdout);
    pid_t pid = fork();
    if (pid < 0) {
        printf("bellows: fork: %s\n", strerror(errno));
        return 126;
    }
    if (pid == 0) {
        execv(path, words);
        printf("bellows: %s: %s\n", words[0], errno == ENOENT ? "orden no encontrada" : strerror(errno));
        fflush(stdout);
        _exit(127);
    }

    int status = 0;
    if (waitpid(pid, &status, 0) != pid) {
        printf("bellows: waitpid: %s\n", strerror(errno));
        return 126;
    }
    if (WIFSIGNALED(status)) {
        printf("bellows: %s terminado por %s\n", words[0], signal_name(WTERMSIG(status)));
        return 128 + WTERMSIG(status);
    }
    return WEXITSTATUS(status);
}

int main(void) {
    char line[MAX_LINE];
    char *words[MAX_WORDS];
    int last = 0;

    printf("Bellows v0 - 'help' para ver las ordenes\n");
    for (;;) {
        printf("bellows> ");
        fflush(stdout);
        if (!fgets(line, sizeof line, stdin)) {
            printf("\n");
            break; /* Ctrl-D */
        }

        int count = split(line, words);
        if (count < 0) {
            printf("bellows: demasiadas palabras (maximo %d)\n", MAX_WORDS - 1);
            continue;
        }
        if (count == 0)
            continue;

        if (strcmp(words[0], "exit") == 0) {
            if (count > 1)
                last = atoi(words[1]);
            break;
        } else if (strcmp(words[0], "help") == 0) {
            printf("  <programa> [args...]   ejecuta /bin/<programa> (o una ruta con '/')\n");
            printf("  pid                    PID de este shell\n");
            printf("  exit [n]               sale (con el codigo n, o el de la ultima orden)\n");
        } else if (strcmp(words[0], "pid") == 0) {
            printf("%d\n", (int)getpid());
        } else {
            last = run(words);
            if (last != 0)
                printf("[codigo %d]\n", last);
        }
    }
    return last;
}
