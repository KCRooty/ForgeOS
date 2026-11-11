/* libctest.c — batería de autocomprobación de la libc sobre ForgeOS.
 *
 * Cada prueba compara contra un resultado conocido y escribe una línea
 * "ok"/"FALLO". El código de salida es el número de pruebas fallidas
 * (0 = todo bien), así que `run /bin/libctest` desde la consola dice de
 * un vistazo si el camino programa -> mlibc -> syscalls -> kernel sigue
 * sano. Úsalo como prueba de regresión tras tocar el kernel.
 */
#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <setjmp.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

#define CHECK(name, cond)                                   \
    do {                                                    \
        if (cond) {                                         \
            printf("  ok     %s\n", name);                  \
        } else {                                            \
            printf("  FALLO  %s  (%s:%d)\n", name, __FILE__, __LINE__); \
            failures++;                                     \
        }                                                   \
    } while (0)

/* --- argv -------------------------------------------------------------- */

static void test_args(int argc, char **argv) {
    printf("argv (%d):", argc);
    for (int i = 0; i < argc; i++)
        printf(" [%s]", argv[i]);
    printf("\n");
    CHECK("argc >= 1 y argv[0] no vacio", argc >= 1 && argv[0] && argv[0][0]);
    CHECK("argv[argc] == NULL", argv[argc] == NULL);
}

/* --- cadenas y formato ------------------------------------------------- */

static int sum_varargs(int n, ...) {
    va_list ap;
    va_start(ap, n);
    int total = 0;
    for (int i = 0; i < n; i++)
        total += va_arg(ap, int);
    va_end(ap);
    return total;
}

static void test_strings(void) {
    char buf[128];

    int n = snprintf(buf, sizeof buf, "%d|%5d|%-5d|%05d|%x|%X|%o|%c|%s|%%", -42, 42, 42, 42, 0xbeef, 0xBEEF, 8, 'F', "os");
    CHECK("snprintf enteros", strcmp(buf, "-42|   42|42   |00042|beef|BEEF|10|F|os|%") == 0 && n == (int)strlen(buf));

    snprintf(buf, sizeof buf, "%ld %lu %lld %zu", -2147483649L, 4294967296UL, -9223372036854775807LL, (size_t)123456789012ULL);
    CHECK("snprintf 64 bits", strcmp(buf, "-2147483649 4294967296 -9223372036854775807 123456789012") == 0);

    snprintf(buf, sizeof buf, "%.3f %8.2f %e %g", 3.14159265, -2.5, 12345.678, 0.0001);
    CHECK("snprintf coma flotante", strcmp(buf, "3.142    -2.50 1.234568e+04 0.0001") == 0);

    n = snprintf(buf, 8, "%s", "truncado de verdad");
    CHECK("snprintf trunca y devuelve la longitud completa", n == 18 && strcmp(buf, "truncad") == 0);

    CHECK("strtol / strtoul", strtol("  -0x1F", NULL, 16) == -31 && strtoul("777", NULL, 8) == 511);
    CHECK("strtod", fabs(strtod("6.02214076e23", NULL) - 6.02214076e23) < 1e9);
    CHECK("atoi", atoi("12345xyz") == 12345);

    char copy[32];
    strcpy(copy, "forge");
    strcat(copy, "-os");
    CHECK("strcpy / strcat / strlen", strcmp(copy, "forge-os") == 0 && strlen(copy) == 8);
    CHECK("strchr / strrchr / strstr", strchr(copy, 'g') == copy + 3 && strrchr(copy, 'o') == copy + 6 && strstr(copy, "e-o") == copy + 4);
    CHECK("memcmp / memmove solapado", (memmove(copy + 2, copy, 6), memcmp(copy, "foforge-", 8) == 0));
    CHECK("toupper / isdigit / isspace", toupper('z') == 'Z' && isdigit('7') && !isdigit('x') && isspace('\t'));
    CHECK("varargs propios", sum_varargs(5, 1, 2, 3, 4, 5) == 15);

    int a, b;
    char word[16];
    CHECK("sscanf", sscanf("640 gib 0x10", "%d %15s %x", &a, word, &b) == 3 && a == 640 && b == 16 && strcmp(word, "gib") == 0);
}

/* --- memoria dinámica -------------------------------------------------- */

static void test_malloc(void) {
    enum { N = 200 };
    static unsigned char *blocks[N];
    int ok = 1;

    /* Muchos bloques de tamaños mezclados, cada uno con su patrón. */
    for (int i = 0; i < N; i++) {
        size_t size = 1 + (size_t)(i * 37 % 2000);
        blocks[i] = malloc(size);
        if (!blocks[i]) { ok = 0; break; }
        memset(blocks[i], i & 0xFF, size);
    }
    /* Libera los pares, comprueba que los impares siguen intactos. */
    for (int i = 0; ok && i < N; i += 2) { free(blocks[i]); blocks[i] = NULL; }
    for (int i = 1; ok && i < N; i += 2) {
        size_t size = 1 + (size_t)(i * 37 % 2000);
        for (size_t j = 0; j < size; j++)
            if (blocks[i][j] != (i & 0xFF)) { ok = 0; break; }
    }
    for (int i = 1; i < N; i += 2) free(blocks[i]);
    CHECK("malloc/free: 200 bloques sin pisarse", ok);

    /* realloc conserva el contenido al crecer de 16 bytes a 1 MiB. */
    size_t size = 16;
    unsigned char *p = malloc(size);
    for (size_t i = 0; i < size; i++) p[i] = (unsigned char)i;
    ok = 1;
    while (size < 1024 * 1024) {
        size_t bigger = size * 4;
        p = realloc(p, bigger);
        if (!p) { ok = 0; break; }
        for (size_t i = 0; i < size; i++)
            if (p[i] != (unsigned char)i) { ok = 0; break; }
        for (size_t i = size; i < bigger; i++) p[i] = (unsigned char)i;
        size = bigger;
    }
    free(p);
    CHECK("realloc conserva el contenido hasta 1 MiB", ok);

    unsigned long *z = calloc(4096, sizeof *z);
    ok = z != NULL;
    for (int i = 0; ok && i < 4096; i++)
        if (z[i] != 0) ok = 0;
    free(z);
    CHECK("calloc devuelve memoria a cero", ok);

    /* 16 MiB de golpe: mucho más que el heap del kernel — sale de mmap(). */
    size_t big = 16u * 1024 * 1024;
    unsigned char *huge = malloc(big);
    ok = huge != NULL;
    if (ok) {
        for (size_t i = 0; i < big; i += 4096) huge[i] = (unsigned char)(i >> 12);
        for (size_t i = 0; i < big; i += 4096)
            if (huge[i] != (unsigned char)(i >> 12)) { ok = 0; break; }
    }
    free(huge);
    CHECK("malloc de 16 MiB, una escritura por pagina", ok);
}

/* --- algoritmos de stdlib ---------------------------------------------- */

static int cmp_int(const void *a, const void *b) {
    int x = *(const int *)a, y = *(const int *)b;
    return (x > y) - (x < y);
}

static void test_stdlib(void) {
    enum { N = 5000 };
    static int v[N];
    unsigned int seed = 0xF0F6E05u;
    long before = 0, after = 0;
    for (int i = 0; i < N; i++) {
        seed = seed * 1664525u + 1013904223u; /* LCG propio: resultado reproducible */
        v[i] = (int)(seed >> 8) % 100000;
        before += v[i];
    }
    qsort(v, N, sizeof v[0], cmp_int);
    int sorted = 1;
    for (int i = 0; i < N; i++) {
        after += v[i];
        if (i > 0 && v[i - 1] > v[i]) sorted = 0;
    }
    CHECK("qsort de 5000 enteros", sorted && before == after);

    int key = v[N / 3];
    int *hit = bsearch(&key, v, N, sizeof v[0], cmp_int);
    CHECK("bsearch encuentra un elemento existente", hit != NULL && *hit == key);

    srand(1234);
    int r1 = rand(), r2 = rand();
    srand(1234);
    CHECK("rand reproducible con la misma semilla", rand() == r1 && rand() == r2 && r1 != r2);
    CHECK("abs / labs / div", abs(-7) == 7 && labs(-70000L) == 70000L && div(17, 5).quot == 3 && div(17, 5).rem == 2);
}

/* --- matemáticas (SSE/x87 de verdad) ----------------------------------- */

static void test_math(void) {
    CHECK("sqrt", fabs(sqrt(2.0) - 1.4142135623730951) < 1e-15);
    CHECK("sin / cos", fabs(sin(1.0) * sin(1.0) + cos(1.0) * cos(1.0) - 1.0) < 1e-15);
    CHECK("exp / log", fabs(log(exp(3.5)) - 3.5) < 1e-14);
    CHECK("pow / floor / ceil / fmod", pow(2.0, 10.0) == 1024.0 && floor(-1.5) == -2.0 && ceil(1.2) == 2.0 && fmod(7.5, 2.0) == 1.5);
    CHECK("atan2 da pi", fabs(4.0 * atan2(1.0, 1.0) - 3.141592653589793) < 1e-15);

    /* Serie de Leibniz con un millón de términos: muchos ciclos de reloj
     * con registros XMM vivos — si un cambio de contexto no los salvara,
     * el resultado saldría distinto. */
    double pi = 0.0, sign = 1.0;
    for (int k = 0; k < 1000000; k++) {
        pi += sign / (2.0 * k + 1.0);
        sign = -sign;
    }
    pi *= 4.0;
    CHECK("serie de Leibniz (1e6 terminos) sobrevive a la preemption", fabs(pi - 3.1415916535897743) < 1e-12);
}

/* --- control de flujo no local ----------------------------------------- */

static jmp_buf jump_target;

static int recurse(int depth) {
    volatile char pad[512]; /* fuerza un frame grande: ~100 KiB de pila en total */
    pad[0] = (char)depth;
    if (depth == 200)
        longjmp(jump_target, 77);
    return recurse(depth + 1) + pad[0];
}

static void test_setjmp(void) {
    volatile int reached = 0;
    int value = setjmp(jump_target);
    if (value == 0) {
        reached = 1;
        recurse(0);
        reached = 2; /* no debe llegar */
    }
    CHECK("setjmp/longjmp desde 200 niveles de recursion", value == 77 && reached == 1);
}

/* --- ficheros (VFS en memoria del kernel) ------------------------------ */

static void test_files(void) {
    const char *path = "libctest.tmp";
    FILE *f = fopen(path, "w");
    int ok = f != NULL;
    if (ok) {
        for (int i = 0; i < 100; i++)
            fprintf(f, "linea %03d de cien\n", i);
        ok = fclose(f) == 0;
    }
    CHECK("fopen(w) + fprintf + fclose", ok);

    f = fopen(path, "r");
    ok = f != NULL;
    int lines = 0;
    char line[64];
    if (ok) {
        while (fgets(line, sizeof line, f)) {
            int n;
            if (sscanf(line, "linea %d de cien", &n) != 1 || n != lines) ok = 0;
            lines++;
        }
        CHECK("fgets relee las 100 lineas en orden", ok && lines == 100);

        /* 100 líneas de 18 bytes: la 50 empieza en el byte 900. */
        ok = fseek(f, 900, SEEK_SET) == 0 && fgets(line, sizeof line, f) && strcmp(line, "linea 050 de cien\n") == 0;
        CHECK("fseek + lectura en medio del fichero", ok);
        ok = fseek(f, 0, SEEK_END) == 0 && ftell(f) == 1800 && fgetc(f) == EOF && feof(f);
        CHECK("fseek(SEEK_END) / ftell / EOF", ok);
        fclose(f);
    } else {
        CHECK("fopen(r) del fichero recien escrito", 0);
    }

    errno = 0;
    f = fopen("no-existe-este-fichero", "r");
    CHECK("fopen de un fichero inexistente falla con ENOENT", f == NULL && errno == ENOENT);
}

int main(int argc, char **argv) {
    printf("libctest - bateria de la libc sobre ForgeOS\n");
    test_args(argc, argv);
    printf("cadenas y formato:\n");
    test_strings();
    printf("memoria dinamica:\n");
    test_malloc();
    printf("stdlib:\n");
    test_stdlib();
    printf("matematicas:\n");
    test_math();
    printf("setjmp/longjmp:\n");
    test_setjmp();
    printf("ficheros:\n");
    test_files();

    if (failures == 0)
        printf("libctest: TODO OK\n");
    else
        printf("libctest: %d prueba(s) FALLIDA(S)\n", failures);
    return failures;
}
