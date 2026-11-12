/* crash.c — provoca a propósito cada fallo que el kernel debe convertir
 * en la muerte del PROCESO (y solo del proceso):
 *
 *   run /bin/crash null     escribe en la dirección 0      -> SIGSEGV (139)
 *   run /bin/crash div0     división entera por cero       -> SIGFPE  (136)
 *   run /bin/crash ud       instrucción ilegal (ud2)       -> SIGILL  (132)
 *   run /bin/crash priv     instrucción privilegiada (hlt) -> SIGSEGV (139)
 *   run /bin/crash stack    recursión infinita             -> SIGSEGV (139)
 *   run /bin/crash abort    abort() de la libc             -> SIGABRT (134)
 *
 * Tras cada uno la consola tiene que seguir viva y `ps` sin rastro del
 * proceso.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* La recursión infinita es justo lo que se quiere aquí. */
#pragma clang diagnostic ignored "-Winfinite-recursion"
static int recurse_forever(volatile int depth) {
    volatile char pad[1024];
    pad[0] = (char)depth;
    return recurse_forever(depth + 1) + pad[0];
}

int main(int argc, char **argv) {
    if (argc < 2) {
        printf("uso: crash null|div0|ud|priv|stack|abort\n");
        return 2;
    }
    const char *mode = argv[1];
    printf("crash: provocando '%s'...\n", mode);

    if (strcmp(mode, "null") == 0) {
        volatile int *p = (volatile int *)0;
        *p = 1;
    } else if (strcmp(mode, "div0") == 0) {
        volatile int zero = 0;
        volatile int result = 100 / zero;
        (void)result;
    } else if (strcmp(mode, "ud") == 0) {
        __asm__ volatile("ud2");
    } else if (strcmp(mode, "priv") == 0) {
        __asm__ volatile("hlt");
    } else if (strcmp(mode, "stack") == 0) {
        return recurse_forever(0);
    } else if (strcmp(mode, "abort") == 0) {
        abort();
    } else {
        printf("crash: modo desconocido '%s'\n", mode);
        return 2;
    }

    printf("crash: SIGO VIVO tras '%s' - el kernel no ha parado el fallo\n", mode);
    return 1;
}
