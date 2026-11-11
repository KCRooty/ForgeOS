/* fpustress.c — comprueba que el estado FPU/SSE de un proceso sobrevive
 * a los cambios de contexto.
 *
 * Calcula durante varios ticks del timer, con valores vivos en registros
 * XMM y en la pila x87 a la vez, y compara con el resultado exacto. Un
 * solo proceso no prueba nada (nadie más toca esos registros): hay que
 * lanzar varios a la vez, cada uno con una semilla distinta —
 *
 *   run /bin/fpustress 1 &
 *   run /bin/fpustress 2 &
 *   run /bin/fpustress 3
 *   wait
 *
 * Si el kernel no salvara/restaurara el estado por tarea, cada proceso
 * acabaría con trozos de la cuenta de otro y el resultado no cuadraría.
 */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    int seed = argc > 1 ? atoi(argv[1]) : 1;
    if (seed < 1 || seed > 1000) {
        printf("uso: fpustress <semilla 1..1000>\n");
        return 2;
    }

    enum { ROUNDS = 40, N = 500000 };
    /* Suma de k = 1..N de (seed + k) en tres aritméticas distintas; las
     * tres tienen resultado entero exacto (muy por debajo de 2^53). */
    const double expected = (double)N * seed + (double)N * (N + 1) / 2.0;

    for (int round = 0; round < ROUNDS; round++) {
        double sse = 0.0;        /* XMM, doble precisión */
        float sse_f = 0.0f;      /* XMM, simple precisión: cuenta de vueltas */
        long double x87 = 0.0L;  /* pila x87, precisión extendida */
        for (int k = 1; k <= N; k++) {
            sse += (double)(seed + k);
            x87 += (long double)(seed + k);
            if (k % 1000 == 0)
                sse_f += 1.0f;
        }
        if (sse != expected || (double)x87 != expected || sse_f != (float)(N / 1000)) {
            printf("fpustress[%d]: FALLO en la ronda %d: sse=%.1f x87=%.1Lf vueltas=%.1f (esperado %.1f)\n",
                   seed, round, sse, x87, (double)sse_f, expected);
            return 1;
        }
    }
    printf("fpustress[%d]: OK - %d rondas, resultado %.1f intacto\n", seed, ROUNDS, expected);
    return 0;
}
