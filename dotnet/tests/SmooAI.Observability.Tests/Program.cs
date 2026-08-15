// Entry point for the test assembly.
//
// `dotnet test` runs through VSTest, which loads this assembly into testhost and
// never calls Main — this exists purely so the assembly can be re-launched as a
// CHILD PROCESS that really dies from an unhandled exception. See CrashChild.cs.
// (`GenerateProgramFile` is disabled in the .csproj so this file is the entry
// point instead of the SDK's generated stub.)
// Synchronous on purpose: an async entry point re-throws the crash on the main
// thread from `GetAwaiter().GetResult()`, by which point the AsyncLocal that
// backs Activity.Current is gone — the "crash inside an ambient Activity" case
// would silently test the no-activity path instead.
return SmooAI.Observability.Tests.CrashChild.Run(args);
