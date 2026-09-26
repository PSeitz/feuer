// Time a bounded-concurrency batch of 500 random 4 KiB O_DIRECT reads.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <linux/aio_abi.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

#define READS 500
#define BLOCK 4096
#define WARMUP 100
#define BATCHES 1000

static void fail(const char *message) {
    perror(message);
    exit(1);
}

static uint64_t now_ns(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC_RAW, &ts)) fail("clock_gettime");
    return (uint64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

static void submit(aio_context_t ctx, struct iocb *cb, uint64_t offset) {
    cb->aio_offset = offset;
    long rc;
    do { rc = syscall(SYS_io_submit, ctx, 1, &cb); } while (rc < 0 && errno == EINTR);
    if (rc != 1) fail("io_submit");
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: batch_reads FILE QD(32|64)\n");
        return 1;
    }
    unsigned qd = (unsigned)strtoul(argv[2], NULL, 10);
    if (qd != 32 && qd != 64) return 1;
    int fd = open(argv[1], O_RDONLY | O_DIRECT);
    if (fd < 0) fail("open");
    struct stat st;
    if (fstat(fd, &st)) fail("fstat");
    if (!S_ISREG(st.st_mode) || st.st_size < BLOCK || st.st_size % BLOCK) return 1;
    uint64_t blocks = st.st_size / BLOCK;
    void *buffers = NULL;
    int error = posix_memalign(&buffers, BLOCK, qd * BLOCK);
    if (error) { errno = error; fail("posix_memalign"); }
    memset(buffers, 0, qd * BLOCK); // Fault buffer pages before timing.
    struct iocb cbs[64] = {0};
    struct io_event events[64];
    aio_context_t ctx = 0;
    if (syscall(SYS_io_setup, qd, &ctx)) fail("io_setup");
    for (unsigned slot = 0; slot < qd; slot++) {
        cbs[slot].aio_data = slot;
        cbs[slot].aio_lio_opcode = IOCB_CMD_PREAD;
        cbs[slot].aio_fildes = fd;
        cbs[slot].aio_buf = (uintptr_t)buffers + slot * BLOCK;
        cbs[slot].aio_nbytes = BLOCK;
    }
    uint64_t rng = 20260926;
    puts("batch,latency_ms");
    for (unsigned batch = 0; batch < WARMUP + BATCHES; batch++) {
        uint64_t offsets[READS];
        for (unsigned i = 0; i < READS; i++) {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            offsets[i] = (rng % blocks) * BLOCK;
        }
        unsigned issued = 0, completed = 0;
        uint64_t start = now_ns();
        for (unsigned slot = 0; slot < qd; slot++) submit(ctx, &cbs[slot], offsets[issued++]);
        while (completed < READS) {
            long n;
            do { n = syscall(SYS_io_getevents, ctx, 1, qd, events, NULL); }
            while (n < 0 && errno == EINTR);
            if (n <= 0) fail("io_getevents");
            for (long i = 0; i < n; i++) {
                if (events[i].res != BLOCK || events[i].res2 != 0 || events[i].data >= qd) {
                    fprintf(stderr, "invalid read completion: %lld / %lld\n",
                            (long long)events[i].res, (long long)events[i].res2);
                    return 1;
                }
                completed++;
                if (issued < READS) submit(ctx, &cbs[events[i].data], offsets[issued++]);
            }
        }
        uint64_t elapsed = now_ns() - start;
        if (issued != READS || completed != READS) return 1;
        if (batch >= WARMUP) printf("%u,%.6f\n", batch - WARMUP, elapsed / 1e6);
    }
    if (syscall(SYS_io_destroy, ctx)) fail("io_destroy");
    free(buffers);
    close(fd);
    return 0;
}
