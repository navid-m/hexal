using System.Runtime.CompilerServices;

// All P/Invoke signatures are blittable and use LibraryImport, so runtime
// marshalling can be disabled. This also keeps NativeAOT output lean.
[assembly: DisableRuntimeMarshalling]
