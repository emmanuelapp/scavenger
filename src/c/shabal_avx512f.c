#include "shabal_avx512f.h"
#include <immintrin.h>
#include <string.h>
#include "common.h"
#include "mshabal_512_avx512f.h"

mshabal512_context global_512;
mshabal512_context_fast global_512_fast;

void init_shabal_avx512f() {
    simd512_mshabal_init(&global_512, 256);
    global_512_fast.out_size = global_512.out_size;
    for (int i = 0; i < 704; i++) global_512_fast.state[i] = global_512.state[i];
    global_512_fast.Whigh = global_512.Whigh;
    global_512_fast.Wlow = global_512.Wlow;
}

void find_best_deadline_avx512f(char *scoops, uint64_t nonce_count, char *gensig,
                                uint64_t *best_deadline, uint64_t *best_offset) {
    uint64_t d0, d1, d2, d3, d4, d5, d6, d7, d8, d9, d10, d11, d12, d13, d14, d15;
    char res0[32], res1[32], res2[32], res3[32], res4[32], res5[32], res6[32], res7[32], res8[32],
        res9[32], res10[32], res11[32], res12[32], res13[32], res14[32], res15[32];
    char end[32];

    end[0] = -128;
    memset(&end[1], 0, 31);

    mshabal512_context_fast x1, x2;
    memcpy(&x2, &global_512_fast,
           sizeof(global_512_fast));  // local copy of global fast contex

    // prepare shabal inputs
    union {
        mshabal_u32 words[64 * MSHABAL512_FACTOR];
        __m512i data[16];
    } u1, u2;

    for (uint64_t i = 0; i < 64 * MSHABAL512_FACTOR / 2; i += 4 * MSHABAL512_FACTOR) {
        size_t o = i / MSHABAL512_FACTOR;
        u1.words[i + 0] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 1] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 2] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 3] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 4] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 5] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 6] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 7] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 8] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 9] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 10] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 11] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 12] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 13] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 14] = *(mshabal_u32 *)(gensig + o);
        u1.words[i + 15] = *(mshabal_u32 *)(gensig + o);
        u2.words[i + 0 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 1 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 2 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 3 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 4 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 5 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 6 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 7 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 8 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 9 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 10 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 11 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 12 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 13 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 14 + 128] = *(mshabal_u32 *)(end + o);
        u2.words[i + 15 + 128] = *(mshabal_u32 *)(end + o);
    }

    for (uint64_t i = 0; i < nonce_count; i += 8) {
        // Inititialise Shabal
        memcpy(&x1, &x2,
               sizeof(x2));  // optimization: mshabal512_init(&x, 256);

        // Load and shuffle data
        // NB: this can be further optimised by preshuffling plot files
        // depending on SIMD length and use avx2 memcpy Did not find a away yet
        // to completely avoid memcpys

        for (uint64_t j = 0; j < 64 * MSHABAL512_FACTOR / 2; j += 4 * MSHABAL512_FACTOR) {
            size_t o = j / MSHABAL512_FACTOR;
            u1.words[j + 0 + 128] = *(mshabal_u32 *)(&scoops[(i + 0) * 64] + o);
            u1.words[j + 1 + 128] = *(mshabal_u32 *)(&scoops[(i + 1) * 64] + o);
            u1.words[j + 2 + 128] = *(mshabal_u32 *)(&scoops[(i + 2) * 64] + o);
            u1.words[j + 3 + 128] = *(mshabal_u32 *)(&scoops[(i + 3) * 64] + o);
            u1.words[j + 4 + 128] = *(mshabal_u32 *)(&scoops[(i + 4) * 64] + o);
            u1.words[j + 5 + 128] = *(mshabal_u32 *)(&scoops[(i + 5) * 64] + o);
            u1.words[j + 6 + 128] = *(mshabal_u32 *)(&scoops[(i + 6) * 64] + o);
            u1.words[j + 7 + 128] = *(mshabal_u32 *)(&scoops[(i + 7) * 64] + o);
            u1.words[j + 8 + 128] = *(mshabal_u32 *)(&scoops[(i + 8) * 64] + o);
            u1.words[j + 9 + 128] = *(mshabal_u32 *)(&scoops[(i + 9) * 64] + o);
            u1.words[j + 10 + 128] = *(mshabal_u32 *)(&scoops[(i + 10) * 64] + o);
            u1.words[j + 11 + 128] = *(mshabal_u32 *)(&scoops[(i + 11) * 64] + o);
            u1.words[j + 12 + 128] = *(mshabal_u32 *)(&scoops[(i + 12) * 64] + o);
            u1.words[j + 13 + 128] = *(mshabal_u32 *)(&scoops[(i + 13) * 64] + o);
            u1.words[j + 14 + 128] = *(mshabal_u32 *)(&scoops[(i + 14) * 64] + o);
            u1.words[j + 15 + 128] = *(mshabal_u32 *)(&scoops[(i + 15) * 64] + o);
            u2.words[j + 0] = *(mshabal_u32 *)(&scoops[(i + 0) * 64 + 32] + o);
            u2.words[j + 1] = *(mshabal_u32 *)(&scoops[(i + 1) * 64 + 32] + o);
            u2.words[j + 2] = *(mshabal_u32 *)(&scoops[(i + 2) * 64 + 32] + o);
            u2.words[j + 3] = *(mshabal_u32 *)(&scoops[(i + 3) * 64 + 32] + o);
            u2.words[j + 4] = *(mshabal_u32 *)(&scoops[(i + 4) * 64 + 32] + o);
            u2.words[j + 5] = *(mshabal_u32 *)(&scoops[(i + 5) * 64 + 32] + o);
            u2.words[j + 6] = *(mshabal_u32 *)(&scoops[(i + 6) * 64 + 32] + o);
            u2.words[j + 7] = *(mshabal_u32 *)(&scoops[(i + 7) * 64 + 32] + o);
            u2.words[j + 8] = *(mshabal_u32 *)(&scoops[(i + 8) * 64 + 32] + o);
            u2.words[j + 9] = *(mshabal_u32 *)(&scoops[(i + 9) * 64 + 32] + o);
            u2.words[j + 10] = *(mshabal_u32 *)(&scoops[(i + 10) * 64 + 32] + o);
            u2.words[j + 11] = *(mshabal_u32 *)(&scoops[(i + 11) * 64 + 32] + o);
            u2.words[j + 12] = *(mshabal_u32 *)(&scoops[(i + 12) * 64 + 32] + o);
            u2.words[j + 13] = *(mshabal_u32 *)(&scoops[(i + 13) * 64 + 32] + o);
            u2.words[j + 14] = *(mshabal_u32 *)(&scoops[(i + 14) * 64 + 32] + o);
            u2.words[j + 15] = *(mshabal_u32 *)(&scoops[(i + 15) * 64 + 32] + o);
        }

        simd512_mshabal_openclose_fast(&x1, &u1, &u2, res0, res1, res2, res3, res4, res5, res6,
                                       res7, res8, res9, res10, res11, res12, res13, res14, res15);

        d0 = *((uint64_t *)res0);
        d1 = *((uint64_t *)res1);
        d2 = *((uint64_t *)res2);
        d3 = *((uint64_t *)res3);
        d4 = *((uint64_t *)res4);
        d5 = *((uint64_t *)res5);
        d6 = *((uint64_t *)res6);
        d7 = *((uint64_t *)res7);
        d8 = *((uint64_t *)res8);
        d9 = *((uint64_t *)res9);
        d10 = *((uint64_t *)res10);
        d11 = *((uint64_t *)res11);
        d12 = *((uint64_t *)res12);
        d13 = *((uint64_t *)res13);
        d14 = *((uint64_t *)res14);
        d15 = *((uint64_t *)res15);

        SET_BEST_DEADLINE(d0, i + 0);
        SET_BEST_DEADLINE(d1, i + 1);
        SET_BEST_DEADLINE(d2, i + 2);
        SET_BEST_DEADLINE(d3, i + 3);
        SET_BEST_DEADLINE(d4, i + 4);
        SET_BEST_DEADLINE(d5, i + 5);
        SET_BEST_DEADLINE(d6, i + 6);
        SET_BEST_DEADLINE(d7, i + 7);
        SET_BEST_DEADLINE(d8, i + 8);
        SET_BEST_DEADLINE(d9, i + 9);
        SET_BEST_DEADLINE(d10, i + 10);
        SET_BEST_DEADLINE(d11, i + 11);
        SET_BEST_DEADLINE(d12, i + 12);
        SET_BEST_DEADLINE(d13, i + 13);
        SET_BEST_DEADLINE(d14, i + 14);
        SET_BEST_DEADLINE(d15, i + 15);
    }
}

