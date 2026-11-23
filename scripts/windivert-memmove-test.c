/*
 * Functional tripwire for issue #7 (second round).
 *
 * The patched windivert.c defines its own memcpy/memset/memmove so the
 * renamed DLL never depends on the CRT. With optimization on, the v143
 * compiler recognizes the byte loops as the builtin memory functions
 * and replaces the loop with a call to the builtin - which resolves to
 * the very function being defined. The first memcpy/memmove with
 * n >= 16 then recurses forever and dies with STATUS_STACK_OVERFLOW
 * on engine start (the 1.5.2 Debug-CRT fix did not stop it: the
 * shipped DLL still crashed the same way, the call landing on a jump
 * stub that jumps straight back into memmove).
 *
 * This test compiles the SAME patched windivert.c with the SAME
 * optimization settings as the DLL build (/O1, intrinsics on) and
 * calls both functions with n >= 16. A crash or a wrong result fails
 * the build. It needs no driver: it exercises the copy functions
 * directly, which is exactly what WinDivertOpen does first (the
 * engine copies its 2 KB filter before any packet is seen).
 *
 * Linking note: the test binary links the CRT, which also defines
 * memcpy/memset/memmove - but object-file definitions win over
 * library definitions, so the calls below resolve to windivert.c's
 * versions. The test main itself is compiled with /Oi- plus
 * #pragma function so its calls are real calls, not intrinsic
 * expansions that would bypass windivert.c's definitions.
 */
#include <stdio.h>
#include <string.h>

#pragma function(memcpy)
#pragma function(memmove)
#pragma function(memset)

int main(void)
{
    unsigned char src[64], dst[64];
    int i, fail = 0;

    for (i = 0; i < 64; i++)
        src[i] = (unsigned char)(i * 7 + 3);

    /* memcpy, n >= 16, non-overlapping: the crashing case. */
    memset(dst, 0, sizeof(dst));
    memcpy(dst, src, 32);
    for (i = 0; i < 32; i++)
        if (dst[i] != src[i])
            fail = 1;

    /* memmove, n >= 16, non-overlapping: the crashing case. */
    memset(dst, 0, sizeof(dst));
    memmove(dst, src, 32);
    for (i = 0; i < 32; i++)
        if (dst[i] != src[i])
            fail = 1;

    /* memmove, overlapping, forward (dst < src). */
    memmove(src, src + 8, 32);
    for (i = 0; i < 32; i++)
        if (src[i] != (unsigned char)((i + 8) * 7 + 3))
            fail = 1;

    /* memmove, overlapping, backward (dst > src). */
    for (i = 0; i < 64; i++)
        src[i] = (unsigned char)(i * 7 + 3);
    memmove(src + 8, src, 32);
    for (i = 0; i < 32; i++)
        if (src[8 + i] != (unsigned char)(i * 7 + 3))
            fail = 1;

    /* memset. */
    memset(dst, 0xA5, 32);
    for (i = 0; i < 32; i++)
        if (dst[i] != 0xA5)
            fail = 1;

    if (fail)
    {
        printf("FAIL: copy result mismatch\n");
        return 1;
    }
    printf("PASS: memcpy/memmove/memset OK\n");
    return 0;
}
