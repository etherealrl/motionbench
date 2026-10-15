#include <ntddk.h>
#include <ntddmou.h>
#include <kbdmou.h>
#pragma warning(push)
#pragma warning(disable: 4324)
#include <wdf.h>
#pragma warning(pop)
#include <initguid.h>
#include <devpkey.h>
#include "protocol.h"
#pragma warning(disable: 4055 4152)

typedef struct _BENCH_CONTEXT {
    WDFSPINLOCK Lock;
    WDFWAITLOCK Lifecycle;
    WDFDEVICE Control;
    ULONG ActiveDevices;
    BENCH_RECORD *Records;
    ULONG Read;
    ULONG Write;
    ULONG Used;
    ULONGLONG Sequence;
    ULONGLONG Batch;
    ULONGLONG Losses;
    LONGLONG Frequency;
    BENCH_DEVICE Devices[BENCH_DEVICE_CAPACITY];
    ULONG DeviceCount;
    ULONGLONG DeviceSequence;
} BENCH_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(BENCH_CONTEXT, BenchContext);
typedef struct _MOUSE_CONTEXT {
    CONNECT_DATA Upper;
    WDFDRIVER Driver;
    ULONGLONG Id;
    BOOLEAN Registered;
} MOUSE_CONTEXT;
WDF_DECLARE_CONTEXT_TYPE_WITH_NAME(MOUSE_CONTEXT, MouseContext);
DRIVER_INITIALIZE DriverEntry;
EVT_WDF_DRIVER_DEVICE_ADD BenchDeviceAdd;
EVT_WDF_IO_QUEUE_IO_INTERNAL_DEVICE_CONTROL BenchInternal;
EVT_WDF_IO_QUEUE_IO_DEVICE_CONTROL BenchRead;
EVT_WDF_OBJECT_CONTEXT_CLEANUP BenchRemove;
static NTSTATUS BenchCreateControl(WDFDRIVER driver);

VOID BenchRemove(WDFOBJECT device)
{
    MOUSE_CONTEXT *mouse = MouseContext(device);
    if (!mouse->Registered) return;
    BENCH_CONTEXT *bench = BenchContext(mouse->Driver);
    WdfWaitLockAcquire(bench->Lifecycle, NULL);
    if (--bench->ActiveDevices == 0 && bench->Control != NULL) {
        WdfObjectDelete(bench->Control);
        bench->Control = NULL;
    }
    WdfWaitLockRelease(bench->Lifecycle);
}

static VOID BenchMouse(PDEVICE_OBJECT device, PMOUSE_INPUT_DATA start,
                       PMOUSE_INPUT_DATA end, PULONG consumed)
{
    MOUSE_CONTEXT *mouse = MouseContext(WdfWdmDeviceGetWdfDeviceHandle(device));
    BENCH_CONTEXT *bench = BenchContext(mouse->Driver);
    LONGLONG before = KeQueryPerformanceCounter(NULL).QuadPart;
    ((PSERVICE_CALLBACK_ROUTINE)mouse->Upper.ClassService)(mouse->Upper.ClassDeviceObject, start, end, consumed);
    LONGLONG after = KeQueryPerformanceCounter(NULL).QuadPart;
    ULONG count = min(*consumed, (ULONG)(end - start));

    WdfSpinLockAcquire(bench->Lock);
    ULONGLONG batch = ++bench->Batch;
    for (ULONG i = 0; i < count; ++i) {
        ULONGLONG sequence = ++bench->Sequence;
        if (bench->Used == BENCH_CAPACITY) { ++bench->Losses; continue; }
        BENCH_RECORD *record = &bench->Records[bench->Write];
        RtlZeroMemory(record, sizeof(*record));
        record->Sequence = sequence; record->Batch = batch;
        record->BeforeQpc = before; record->AfterQpc = after;
        record->Device = mouse->Id; record->Index = i; record->Count = count;
        record->Flags = start[i].Flags; record->Buttons = start[i].ButtonFlags;
        record->ButtonData = start[i].ButtonData; record->Unit = start[i].UnitId;
        record->Dx = start[i].LastX; record->Dy = start[i].LastY;
        record->Extra = start[i].ExtraInformation;
        bench->Write = (bench->Write + 1) % BENCH_CAPACITY;
        ++bench->Used;
    }
    WdfSpinLockRelease(bench->Lock);
}

VOID BenchInternal(WDFQUEUE queue, WDFREQUEST request, size_t outBytes,
                   size_t inBytes, ULONG code)
{
    UNREFERENCED_PARAMETER(outBytes); UNREFERENCED_PARAMETER(inBytes);
    WDFDEVICE device = WdfIoQueueGetDevice(queue);
    MOUSE_CONTEXT *mouse = MouseContext(device);
    if (code == IOCTL_INTERNAL_MOUSE_CONNECT) {
        CONNECT_DATA *connect;
        if (mouse->Upper.ClassService != NULL) {
            WdfRequestComplete(request, STATUS_SHARING_VIOLATION); return;
        }
        NTSTATUS status = WdfRequestRetrieveInputBuffer(request, sizeof(*connect), (PVOID*)&connect, NULL);
        if (!NT_SUCCESS(status)) { WdfRequestComplete(request, status); return; }
        mouse->Upper = *connect;
        connect->ClassDeviceObject = WdfDeviceWdmGetDeviceObject(device);
        connect->ClassService = BenchMouse;
    }
    WDF_REQUEST_SEND_OPTIONS options;
    WDF_REQUEST_SEND_OPTIONS_INIT(&options, WDF_REQUEST_SEND_OPTION_SEND_AND_FORGET);
    if (!WdfRequestSend(request, WdfDeviceGetIoTarget(device), &options)) {
        WdfRequestComplete(request, WdfRequestGetStatus(request));
    }
}

VOID BenchRead(WDFQUEUE queue, WDFREQUEST request, size_t outBytes,
               size_t inBytes, ULONG code)
{
    UNREFERENCED_PARAMETER(inBytes);
    BENCH_CONTEXT *bench = BenchContext(WdfDeviceGetDriver(WdfIoQueueGetDevice(queue)));
    if (code != BENCH_READ && code != BENCH_DEVICES) {
        WdfRequestComplete(request, STATUS_INVALID_DEVICE_REQUEST); return;
    }
    BENCH_HEADER *header;
    NTSTATUS status = WdfRequestRetrieveOutputBuffer(request, sizeof(*header), (PVOID*)&header, NULL);
    if (!NT_SUCCESS(status)) { WdfRequestComplete(request, status); return; }
    size_t itemSize = code == BENCH_READ ? sizeof(BENCH_RECORD) : sizeof(BENCH_DEVICE);
    ULONG capacity = (ULONG)min((outBytes - sizeof(*header)) / itemSize, 256);
    WdfSpinLockAcquire(bench->Lock);
    header->Version = BENCH_VERSION; header->Frequency = bench->Frequency;
    header->Losses = bench->Losses; header->NextSequence = bench->Sequence + 1;
    header->Count = min(capacity, code == BENCH_READ ? bench->Used : bench->DeviceCount);
    if (code == BENCH_READ) {
        BENCH_RECORD *records = (BENCH_RECORD*)(header + 1);
        for (ULONG i = 0; i < header->Count; ++i) {
            records[i] = bench->Records[bench->Read];
            bench->Read = (bench->Read + 1) % BENCH_CAPACITY; --bench->Used;
        }
    } else {
        RtlCopyMemory(header + 1, bench->Devices, header->Count * sizeof(BENCH_DEVICE));
    }
    WdfSpinLockRelease(bench->Lock);
    WdfRequestCompleteWithInformation(request, STATUS_SUCCESS, sizeof(*header) + header->Count * itemSize);
}

NTSTATUS BenchDeviceAdd(WDFDRIVER driver, PWDFDEVICE_INIT init)
{
    WDF_OBJECT_ATTRIBUTES attributes;
    WDFDEVICE device;
    WdfFdoInitSetFilter(init);
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, MOUSE_CONTEXT);
    attributes.EvtCleanupCallback = BenchRemove;
    NTSTATUS status = WdfDeviceCreate(&init, &attributes, &device);
    if (!NT_SUCCESS(status)) return status;
    MOUSE_CONTEXT *mouse = MouseContext(device);
    mouse->Driver = driver;
    BENCH_CONTEXT *bench = BenchContext(driver);
    WdfWaitLockAcquire(bench->Lifecycle, NULL);
    if (bench->Control == NULL) (void)BenchCreateControl(driver);
    ++bench->ActiveDevices; mouse->Registered = TRUE;
    WdfWaitLockRelease(bench->Lifecycle);
    WdfSpinLockAcquire(bench->Lock);
    mouse->Id = ++bench->DeviceSequence;
    ULONG index = (ULONG)((mouse->Id - 1) % BENCH_DEVICE_CAPACITY);
    if (bench->DeviceCount < BENCH_DEVICE_CAPACITY) ++bench->DeviceCount;
    else ++bench->Losses;
    RtlZeroMemory(&bench->Devices[index], sizeof(BENCH_DEVICE));
    bench->Devices[index].Id = mouse->Id;
    WdfSpinLockRelease(bench->Lock);
    WDF_DEVICE_PROPERTY_DATA property;
    WDF_DEVICE_PROPERTY_DATA_INIT(&property, &DEVPKEY_Device_InstanceId);
    ULONG required = 0;
    DEVPROPTYPE type;
    WCHAR instance[256] = {0};
    status = WdfDeviceQueryPropertyEx(device, &property, sizeof(instance), instance, &required, &type);
    if (NT_SUCCESS(status) && type == DEVPROP_TYPE_STRING) {
        WdfSpinLockAcquire(bench->Lock);
        RtlCopyMemory(bench->Devices[index].Instance, instance, sizeof(instance));
        WdfSpinLockRelease(bench->Lock);
    }
    WDF_IO_QUEUE_CONFIG config;
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&config, WdfIoQueueDispatchParallel);
    config.EvtIoInternalDeviceControl = BenchInternal;
    return WdfIoQueueCreate(device, &config, WDF_NO_OBJECT_ATTRIBUTES, WDF_NO_HANDLE);
}

NTSTATUS DriverEntry(PDRIVER_OBJECT object, PUNICODE_STRING registry)
{
    WDF_DRIVER_CONFIG config;
    WDF_OBJECT_ATTRIBUTES attributes;
    WDFDRIVER driver;
    WDF_DRIVER_CONFIG_INIT(&config, BenchDeviceAdd);
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, BENCH_CONTEXT);
    NTSTATUS status = WdfDriverCreate(object, registry, &attributes, &config, &driver);
    if (!NT_SUCCESS(status)) return status;
    BENCH_CONTEXT *bench = BenchContext(driver);
    LARGE_INTEGER frequency;
    KeQueryPerformanceCounter(&frequency); bench->Frequency = frequency.QuadPart;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes); attributes.ParentObject = driver;
    status = WdfSpinLockCreate(&attributes, &bench->Lock);
    if (!NT_SUCCESS(status)) return status;
    status = WdfWaitLockCreate(&attributes, &bench->Lifecycle);
    if (!NT_SUCCESS(status)) return status;
    WDFMEMORY memory;
    status = WdfMemoryCreate(&attributes, NonPagedPoolNx, 'bnoM',
        BENCH_CAPACITY * sizeof(BENCH_RECORD), &memory, (PVOID*)&bench->Records);
    return status;
}

static NTSTATUS BenchCreateControl(WDFDRIVER driver)
{
    DECLARE_CONST_UNICODE_STRING(sddl, L"D:P(A;;GA;;;SY)(A;;GA;;;BA)");
    PWDFDEVICE_INIT init = WdfControlDeviceInitAllocate(driver, &sddl);
    if (init == NULL) return STATUS_INSUFFICIENT_RESOURCES;
    DECLARE_CONST_UNICODE_STRING(name, L"\\Device\\MotionBenchObserver");
    DECLARE_CONST_UNICODE_STRING(link, L"\\DosDevices\\MotionBenchObserver");
    NTSTATUS status = WdfDeviceInitAssignName(init, &name);
    WDFDEVICE control = NULL;
    if (NT_SUCCESS(status)) {
        WdfDeviceInitSetExclusive(init, TRUE);
        status = WdfDeviceCreate(&init, WDF_NO_OBJECT_ATTRIBUTES, &control);
    }
    if (!NT_SUCCESS(status)) { if (init != NULL) WdfDeviceInitFree(init); return status; }
    status = WdfDeviceCreateSymbolicLink(control, &link);
    if (!NT_SUCCESS(status)) { WdfObjectDelete(control); return status; }
    WDF_IO_QUEUE_CONFIG queueConfig;
    WDF_IO_QUEUE_CONFIG_INIT_DEFAULT_QUEUE(&queueConfig, WdfIoQueueDispatchSequential);
    queueConfig.EvtIoDeviceControl = BenchRead;
    status = WdfIoQueueCreate(control, &queueConfig, WDF_NO_OBJECT_ATTRIBUTES, WDF_NO_HANDLE);
    if (NT_SUCCESS(status)) {
        WdfControlFinishInitializing(control);
        BenchContext(driver)->Control = control;
    } else { WdfObjectDelete(control); }
    return status;
}
