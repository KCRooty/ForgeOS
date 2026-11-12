/* proctest.c — batería de procesos sobre ForgeOS: fork/exec/wait/kill
 * desde C, y que el kernel rechace punteros que no son del proceso.
 *
 * Mismo formato que libctest: una línea "ok"/"FALLO" por prueba y el
 * número de fallos como código de salida.
 *
 *   run /bin/proctest
 *
 * Con argumentos hace de hijo de sus propias pruebas de exec:
 *   proctest --args <a> <b>...   sale con argc*10 + strlen(último argumento)
 */
#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

static int failures;

#define CHECK(name, cond)                                   \
    do {                                                    \
        if (cond) {                                         \
            printf("  ok     %s\n", name);                  \
        } else {                                            \
            printf("  FALLO  %s  (%s:%d, errno=%d)\n", name, __FILE__, __LINE__, errno); \
            failures++;                                     \
        }                                                   \
        fflush(stdout);                                     \
    } while (0)

/* fork() con la salida vaciada antes: lo que quedara en el buffer de
 * stdio se duplicaría en el hijo. */
static pid_t fork_clean(void) {
    fflush(stdout);
    return fork();
}

/* Espera a `pid` y devuelve su wait status (o -1). */
static int wait_for(pid_t pid) {
    int status = -1;
    if (waitpid(pid, &status, 0) != pid)
        return -1;
    return status;
}

static int shared_counter = 100;

static void test_fork_basics(void) {
    pid_t me = getpid();
    CHECK("getpid() > 0 y getppid() >= 0", me > 0 && getppid() >= 0);

    /* Variables vivas en registros a través de fork(): el compilador las
     * deja en rbx/r12-r15, y el hijo tiene que verlas igual que el padre. */
    long a = 0x1111111111111111L, b = 0x2222222222222222L, c = 0x3333333333333333L;
    double pi = 3.14159265358979;
    pid_t pid = fork_clean();
    if (pid == 0) {
        int ok = a == 0x1111111111111111L && b == 0x2222222222222222L && c == 0x3333333333333333L
                 && pi > 3.14159 && pi < 3.14160 && getppid() == me && getpid() != me;
        shared_counter = 999; /* solo en la copia del hijo */
        _exit(ok ? 7 : 8);
    }
    int status = wait_for(pid);
    CHECK("fork: el hijo conserva registros, FPU y ve a su padre", pid > 0 && WIFEXITED(status) && WEXITSTATUS(status) == 7);
    CHECK("fork: la memoria del hijo es una copia, no la del padre", shared_counter == 100);
    CHECK("fork: el padre conserva sus propios registros", a + b == c);

    /* malloc en el hijo: necesita haber heredado el estado del heap. */
    char *before = malloc(1000);
    strcpy(before, "reservado antes del fork");
    pid = fork_clean();
    if (pid == 0) {
        char *after = malloc(200000);
        int ok = after != NULL && strcmp(before, "reservado antes del fork") == 0;
        if (ok) {
            memset(after, 0x5A, 200000);
            ok = after[0] == 0x5A && after[199999] == 0x5A && strcmp(before, "reservado antes del fork") == 0;
        }
        _exit(ok ? 0 : 1);
    }
    status = wait_for(pid);
    CHECK("fork: malloc funciona en el hijo sin pisar lo heredado", WIFEXITED(status) && WEXITSTATUS(status) == 0);
    free(before);
}

static void test_many_children(void) {
    enum { N = 8 };
    pid_t pids[N];
    int ok = 1;

    /* Ocho fork() seguidos sin ceder el turno entre medias. */
    for (int i = 0; i < N; i++) {
        pids[i] = fork_clean();
        if (pids[i] == 0) {
            volatile unsigned long spin = 0;
            for (unsigned long k = 0; k < 200000UL * (unsigned long)(N - i); k++)
                spin += k; /* los primeros tardan más: terminan desordenados */
            _exit(20 + i);
        }
        if (pids[i] < 0) ok = 0;
    }
    /* Se recogen por PID, en orden inverso al de creación. */
    for (int i = N - 1; i >= 0; i--) {
        int status = wait_for(pids[i]);
        if (!WIFEXITED(status) || WEXITSTATUS(status) != 20 + i) ok = 0;
    }
    CHECK("8 hijos a la vez, cada uno con su codigo, recogidos por PID", ok);

    errno = 0;
    int status;
    CHECK("waitpid sin hijos falla con ECHILD", waitpid(-1, &status, 0) == -1 && errno == ECHILD);

    /* waitpid(-1): cualquiera. */
    pid_t x = fork_clean();
    if (x == 0) _exit(1);
    pid_t y = fork_clean();
    if (y == 0) _exit(2);
    int sum = 0, got = 0;
    for (int i = 0; i < 2; i++) {
        pid_t who = waitpid(-1, &status, 0);
        if ((who == x || who == y) && WIFEXITED(status)) {
            sum += WEXITSTATUS(status);
            got++;
        }
    }
    CHECK("waitpid(-1) recoge a los dos hijos", got == 2 && sum == 3);
}

static void test_signals(void) {
    int status;

    /* Hijo que no termina solo: WNOHANG dice "todavía no", y kill() lo para. */
    pid_t pid = fork_clean();
    if (pid == 0) {
        for (;;)
            sched_yield();
    }
    CHECK("waitpid(WNOHANG) devuelve 0 con el hijo vivo", waitpid(pid, &status, WNOHANG) == 0);
    CHECK("kill(hijo, SIGTERM)", kill(pid, SIGTERM) == 0);
    status = wait_for(pid);
    CHECK("el hijo muere por SIGTERM", WIFSIGNALED(status) && WTERMSIG(status) == SIGTERM);
    errno = 0;
    CHECK("kill a un PID que no existe falla con ESRCH", kill(pid, SIGTERM) == -1 && errno == ESRCH);

    pid = fork_clean();
    if (pid == 0) {
        volatile int *p = (volatile int *)0;
        *p = 1;
        _exit(0);
    }
    status = wait_for(pid);
    CHECK("un hijo que escribe en NULL muere por SIGSEGV", WIFSIGNALED(status) && WTERMSIG(status) == SIGSEGV);

    pid = fork_clean();
    if (pid == 0) {
        abort();
    }
    status = wait_for(pid);
    CHECK("abort() en el hijo es SIGABRT", WIFSIGNALED(status) && WTERMSIG(status) == SIGABRT);

    /* exit(139) NO es lo mismo que morir por SIGSEGV. */
    pid = fork_clean();
    if (pid == 0) _exit(139);
    status = wait_for(pid);
    CHECK("exit(139) se distingue de una senal", WIFEXITED(status) && WEXITSTATUS(status) == 139 && !WIFSIGNALED(status));
}

static void test_exec(const char *self) {
    int status;

    pid_t pid = fork_clean();
    if (pid == 0) {
        char *argv[] = {"hello", NULL};
        execv("/bin/hello", argv);
        _exit(98); /* solo si execv falla */
    }
    status = wait_for(pid);
    CHECK("fork + execv(/bin/hello) desde el disco: sale con 42", WIFEXITED(status) && WEXITSTATUS(status) == 42);

    pid = fork_clean();
    if (pid == 0) {
        char *argv[] = {"proctest", "--args", "uno", "dos", "tres-con-guion", NULL};
        execv(self, argv);
        _exit(98);
    }
    status = wait_for(pid);
    /* 5 argumentos, el último de 14 letras: 5*10 + 14. */
    CHECK("execv entrega argv completo al programa nuevo", WIFEXITED(status) && WEXITSTATUS(status) == 64);

    pid = fork_clean();
    if (pid == 0) {
        char *argv[] = {"nada", NULL};
        execv("/bin/no-existe", argv);
        _exit(errno == ENOENT ? 97 : 98);
    }
    status = wait_for(pid);
    CHECK("execv de un fichero inexistente vuelve con ENOENT", WIFEXITED(status) && WEXITSTATUS(status) == 97);

    pid = fork_clean();
    if (pid == 0) {
        char *argv[] = {"texto", NULL};
        execv("/hello.txt", argv); /* existe, pero no es un ELF */
        _exit(errno == ENOEXEC ? 97 : 98);
    }
    status = wait_for(pid);
    CHECK("execv de algo que no es un ELF vuelve con ENOEXEC", WIFEXITED(status) && WEXITSTATUS(status) == 97);
}

/* Punteros que no pertenecen al proceso: el kernel debe devolver EFAULT,
 * no leer ni escribir ahí (y desde luego no colgarse). */
static void test_bad_pointers(void) {
    void *kernel_memory = (void *)0x200000;       /* dentro de la imagen del kernel */
    void *unmapped = (void *)0x00007F0000000000;  /* mitad de usuario, sin mapear */
    void *noncanonical = (void *)0x8000000000000000;
    char buf[16];

    errno = 0;
    CHECK("write() desde memoria del kernel -> EFAULT", write(1, kernel_memory, 16) == -1 && errno == EFAULT);
    errno = 0;
    CHECK("write() desde una direccion sin mapear -> EFAULT", write(1, unmapped, 16) == -1 && errno == EFAULT);
    errno = 0;
    CHECK("write() desde una direccion no canonica -> EFAULT", write(1, noncanonical, 16) == -1 && errno == EFAULT);
    errno = 0;
    CHECK("write() que se sale de la pila por arriba -> EFAULT", write(1, buf, 1024 * 1024) == -1 && errno == EFAULT);
    errno = 0;
    CHECK("open() con la ruta en memoria del kernel -> EFAULT", open(kernel_memory, O_RDONLY) == -1 && errno == EFAULT);

    int fd = open("libctest.tmp", O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) {
        write(fd, "contenido", 9);
        close(fd);
    }
    fd = open("libctest.tmp", O_RDONLY);
    errno = 0;
    CHECK("read() hacia memoria del kernel -> EFAULT", fd >= 0 && read(fd, kernel_memory, 9) == -1 && errno == EFAULT);
    /* Código del propio programa: mapeado, pero de solo lectura. */
    errno = 0;
    CHECK("read() hacia una pagina de solo lectura -> EFAULT", fd >= 0 && read(fd, (void *)&test_bad_pointers, 9) == -1 && errno == EFAULT);
    CHECK("read() normal sigue funcionando despues", fd >= 0 && read(fd, buf, 9) == 9 && memcmp(buf, "contenido", 9) == 0);
    if (fd >= 0) close(fd);

    errno = 0;
    CHECK("waitpid() con status en memoria del kernel -> EFAULT", waitpid(-1, kernel_memory, WNOHANG) == -1 && (errno == EFAULT || errno == ECHILD));

    pid_t pid = fork_clean();
    if (pid == 0) {
        execv(kernel_memory, kernel_memory);
        _exit(errno == EFAULT ? 97 : 98);
    }
    int status = wait_for(pid);
    CHECK("execv() con punteros del kernel -> EFAULT", WIFEXITED(status) && WEXITSTATUS(status) == 97);
}

/* Un nieto que sobrevive a su padre: nadie le hará wait(). No debe
 * quedarse como zombie para siempre (lo recoge la consola). */
static void test_orphan(void) {
    pid_t pid = fork_clean();
    if (pid == 0) {
        if (fork() == 0) {
            for (int i = 0; i < 5; i++)
                sched_yield();
            _exit(0);
        }
        _exit(5); /* el padre del nieto se va primero */
    }
    int status = wait_for(pid);
    CHECK("un proceso puede morir dejando un hijo vivo", WIFEXITED(status) && WEXITSTATUS(status) == 5);
}

int main(int argc, char **argv) {
    if (argc >= 2 && strcmp(argv[1], "--args") == 0)
        return argc * 10 + (int)strlen(argv[argc - 1]);

    printf("proctest - procesos sobre ForgeOS (PID %d)\n", (int)getpid());
    printf("fork:\n");
    test_fork_basics();
    printf("varios hijos y waitpid:\n");
    test_many_children();
    printf("senales:\n");
    test_signals();
    printf("exec:\n");
    test_exec(argv[0]);
    printf("punteros invalidos:\n");
    test_bad_pointers();
    printf("huerfanos:\n");
    test_orphan();

    if (failures == 0)
        printf("proctest: TODO OK\n");
    else
        printf("proctest: %d prueba(s) FALLIDA(S)\n", failures);
    return failures;
}
