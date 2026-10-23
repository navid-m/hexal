namespace Hexal.Dap;

public sealed record DapScope(
    string Name,
    int VariablesReference,
    bool Expensive);

public sealed record DapVariable(
    string Name,
    string Value,
    string Type,
    int VariablesReference,
    string EvaluateName);

public sealed record DapScopeVariables(
    string Name,
    IReadOnlyList<DapVariable> Variables);

public sealed record DapWatchValue(
    string Expression,
    string Value,
    string Type,
    string Error);
