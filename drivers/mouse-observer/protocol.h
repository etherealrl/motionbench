#pragma once
#define BENCH_VERSION 1
#define BENCH_READ CTL_CODE(FILE_DEVICE_UNKNOWN, 0x800, METHOD_BUFFERED, FILE_READ_DATA)
#define BENCH_DEVICES CTL_CODE(FILE_DEVICE_UNKNOWN, 0x801, METHOD_BUFFERED, FILE_READ_DATA)
#define BENCH_CAPACITY 32768
#define BENCH_DEVICE_CAPACITY 32
#pragma pack(push, 8)
typedef struct _BENCH_HEADER {
    ULONG Version;
    ULONG Count;
    LONGLONG Frequency;
    ULONGLONG Losses;
    ULONGLONG NextSequence;
} BENCH_HEADER;
typedef struct _BENCH_RECORD {
    ULONGLONG Sequence;
    ULONGLONG Batch;
    LONGLONG BeforeQpc;
    LONGLONG AfterQpc;
    ULONGLONG Device;
    ULONG Index;
    ULONG Count;
    USHORT Flags;
    USHORT Buttons;
    USHORT ButtonData;
    USHORT Unit;
    LONG Dx;
    LONG Dy;
    ULONG Extra;
    ULONG Reserved;
} BENCH_RECORD;
typedef struct _BENCH_DEVICE {
    ULONGLONG Id;
    WCHAR Instance[256];
} BENCH_DEVICE;
#pragma pack(pop)
C_ASSERT(sizeof(BENCH_HEADER) == 32);
C_ASSERT(sizeof(BENCH_RECORD) == 72);
