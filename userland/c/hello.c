/* hello.c — primer programa C de ForgeOS enlazado contra mlibc (M11).
 *
 * Cada línea ejercita una pieza distinta del camino libc -> kernel:
 * stdio con buffer (write), el frame inicial de la pila (argc/argv),
 * malloc (mmap anónimo), coma flotante (SSE habilitado en el kernel) y
 * el valor de retorno de main() (exit).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    printf("hello, forge! (C + mlibc)\n");
    printf("argc=%d argv[0]=%s\n", argc, argc > 0 ? argv[0] : "(null)");

    /* malloc + memoria de verdad: 64 KiB escritos y comprobados. */
    size_t n = 64 * 1024;
    unsigned char *buf = malloc(n);
    if (!buf) {
        printf("malloc(%zu) fallo\n", n);
        return 1;
    }
    for (size_t i = 0; i < n; i++)
        buf[i] = (unsigned char)(i * 7 + 3);
    unsigned long sum = 0;
    for (size_t i = 0; i < n; i++)
        sum += buf[i];
    printf("malloc(%zu) OK, checksum=%lu (esperado 8355840)\n", n, sum);
    free(buf);

    /* Coma flotante: necesita SSE activo y preservado entre cambios de contexto. */
    double x = 0.0;
    for (int i = 1; i <= 100; i++)
        x += 1.0 / (double)i;
    printf("H(100)=%.6f (esperado 5.187378)\n", x);

    char msg[64];
    snprintf(msg, sizeof msg, "%s-%04x-%s", "forge", 0xBEEF, "os");
    printf("snprintf: %s (len=%zu)\n", msg, strlen(msg));

    return 42;
}
