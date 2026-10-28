local B = {}

B.namespaces = {
    "System",
    "System.Collections",
    "System.Collections.Concurrent",
    "System.Collections.Generic",
    "System.Diagnostics",
    "System.Globalization",
    "System.IO",
    "System.Linq",
    "System.Net",
    "System.Net.Http",
    "System.Reflection",
    "System.Runtime.CompilerServices",
    "System.Security.Cryptography",
    "System.Text",
    "System.Text.Json",
    "System.Threading",
    "System.Threading.Tasks",
    "Microsoft.Extensions.DependencyInjection",
    "Microsoft.Extensions.Logging",
}

local function member(name, kind, result_type, signature, documentation, extra)
    local value = {
        name = name,
        kind = kind,
        type = result_type or "",
        signature = signature or name,
        documentation = documentation or "",
    }
    if extra then
        for key, item in pairs(extra) do
            value[key] = item
        end
    end
    return value
end

local function method(name, result_type, signature, documentation)
    return member(name, "method", result_type, signature, documentation)
end

local function static_method(name, result_type, signature, documentation)
    return member(name, "method", result_type, signature, documentation,
        { static = true })
end

local function property(name, result_type, documentation, extra)
    return member(name, "property", result_type, name, documentation, extra)
end

local function static_property(name, result_type, documentation)
    return member(name, "property", result_type, name, documentation,
        { static = true })
end

local function field(name, result_type, documentation, extra)
    return member(name, "field", result_type, name, documentation, extra)
end

local function static_field(name, result_type, documentation)
    return member(name, "field", result_type, name, documentation,
        { static = true })
end

local function constructor(signature, documentation)
    return member("new", "constructor", "", signature, documentation)
end

local object_members = {
    method("ToString", "string", "ToString()", "Returns a string that represents the current object."),
    method("Equals", "bool", "Equals(object? obj)", "Determines whether the specified object is equal to the current object."),
    method("GetHashCode", "int", "GetHashCode()", "Returns a hash code for the current object."),
    method("GetType", "Type", "GetType()", "Gets the runtime type of the current instance."),
}

local function with_object(members)
    local result = {}
    local seen = {}
    for _, item in ipairs(members or {}) do
        result[#result + 1] = item
        seen[item.name] = true
    end
    for _, item in ipairs(object_members) do
        if not seen[item.name] then
            result[#result + 1] = item
        end
    end
    return result
end

local function numeric_members(type_name)
    return with_object({
        static_field("MaxValue", type_name, "The largest possible value of this type."),
        static_field("MinValue", type_name, "The smallest possible value of this type."),
        static_method("Parse", type_name, "Parse(string s)",
            "Converts the string representation to a value."),
        static_method("TryParse", "bool", "TryParse(string s, out " .. type_name .. " result)",
            "Attempts to convert the string representation to a value."),
        method("CompareTo", "int", "CompareTo(" .. type_name .. " value)",
            "Compares this instance with another value."),
    })
end

local enum_members = {
    method("HasFlag", "bool", "HasFlag(Enum flag)", "Determines whether one or more bit fields are set."),
    method("ToString", "string", "ToString()", "Converts the value to its string representation."),
    static_method("Parse", "object", "Parse(Type enumType, string value)",
        "Converts the string representation to an enum value."),
    static_method("TryParse", "bool", "TryParse(Type enumType, string value, out object result)",
        "Attempts to convert the string representation to an enum value."),
    static_method("GetValues", "Array", "GetValues(Type enumType)",
        "Retrieves an array of the values of the constants in an enum."),
    static_method("GetNames", "string[]", "GetNames(Type enumType)",
        "Retrieves an array of the names of the constants in an enum."),
}

local enumerable_members = {
    method("GetEnumerator", "IEnumerator<T>", "GetEnumerator()", "Returns an enumerator that iterates through the collection."),
    method("ToList", "List<T>", "ToList()", "Creates a List<T> from the sequence."),
    method("ToArray", "T[]", "ToArray()", "Creates an array from the sequence."),
    method("Count", "int", "Count()", "Returns the number of elements in the sequence."),
    method("Any", "bool", "Any()", "Determines whether the sequence contains any elements."),
    method("All", "bool", "All(Func<T, bool> predicate)", "Determines whether all elements satisfy a condition."),
    method("Where", "IEnumerable<T>", "Where(Func<T, bool> predicate)", "Filters a sequence of values based on a predicate."),
    method("Select", "IEnumerable<TResult>", "Select<TResult>(Func<T, TResult> selector)", "Projects each element into a new form."),
    method("SelectMany", "IEnumerable<TResult>", "SelectMany<TResult>(Func<T, IEnumerable<TResult>> selector)", "Projects and flattens sequences."),
    method("First", "T", "First()", "Returns the first element of the sequence."),
    method("FirstOrDefault", "T", "FirstOrDefault()", "Returns the first element or a default value."),
    method("Single", "T", "Single()", "Returns the only element of the sequence."),
    method("SingleOrDefault", "T", "SingleOrDefault()", "Returns the only element or a default value."),
    method("Last", "T", "Last()", "Returns the last element of the sequence."),
    method("LastOrDefault", "T", "LastOrDefault()", "Returns the last element or a default value."),
    method("OrderBy", "IOrderedEnumerable<T>", "OrderBy<TKey>(Func<T, TKey> keySelector)", "Sorts the elements in ascending order."),
    method("OrderByDescending", "IOrderedEnumerable<T>", "OrderByDescending<TKey>(Func<T, TKey> keySelector)", "Sorts the elements in descending order."),
    method("ThenBy", "IOrderedEnumerable<T>", "ThenBy<TKey>(Func<T, TKey> keySelector)", "Performs a subsequent ascending sort."),
    method("GroupBy", "IEnumerable<IGrouping<TKey, T>>", "GroupBy<TKey>(Func<T, TKey> keySelector)", "Groups elements by a key."),
    method("Skip", "IEnumerable<T>", "Skip(int count)", "Bypasses a number of elements."),
    method("Take", "IEnumerable<T>", "Take(int count)", "Returns a number of contiguous elements."),
    method("Distinct", "IEnumerable<T>", "Distinct()", "Returns distinct elements."),
    method("Concat", "IEnumerable<T>", "Concat(IEnumerable<T> second)", "Concatenates two sequences."),
    method("Union", "IEnumerable<T>", "Union(IEnumerable<T> second)", "Produces the set union of two sequences."),
    method("Intersect", "IEnumerable<T>", "Intersect(IEnumerable<T> second)", "Produces the set intersection of two sequences."),
    method("Except", "IEnumerable<T>", "Except(IEnumerable<T> second)", "Produces the set difference of two sequences."),
    method("Sum", "double", "Sum()", "Computes the sum of a sequence of numeric values."),
    method("Min", "T", "Min()", "Returns the minimum value in a sequence."),
    method("Max", "T", "Max()", "Returns the maximum value in a sequence."),
    method("Average", "double", "Average()", "Computes the average of a sequence of numeric values."),
    method("Aggregate", "T", "Aggregate(Func<T, T, T> func)", "Applies an accumulator function over a sequence."),
    method("Contains", "bool", "Contains(T value)", "Determines whether a sequence contains a value."),
    method("ElementAt", "T", "ElementAt(int index)", "Returns the element at a specified index."),
    method("Reverse", "IEnumerable<T>", "Reverse()", "Inverts the order of the elements."),
    method("Cast", "IEnumerable<TResult>", "Cast<TResult>()", "Casts the elements to a specified type."),
    method("OfType", "IEnumerable<TResult>", "OfType<TResult>()", "Filters elements by type."),
    method("Zip", "IEnumerable<TResult>", "Zip<TSecond, TResult>(IEnumerable<TSecond> second, Func<T, TSecond, TResult> resultSelector)", "Combines two sequences."),
    method("Chunk", "IEnumerable<T[]>", "Chunk(int size)", "Splits the sequence into chunks."),
}

local function collection_members()
    local result = {
        property("Count", "int", "Gets the number of elements contained in the collection.", { readonly = true }),
        method("Contains", "bool", "Contains(T item)", "Determines whether the collection contains a value."),
        method("CopyTo", "void", "CopyTo(T[] array, int arrayIndex)", "Copies the elements to an array."),
        method("Clear", "void", "Clear()", "Removes all elements."),
    }
    for _, item in ipairs(enumerable_members) do
        result[#result + 1] = item
    end
    return with_object(result)
end

local function list_members()
    local result = collection_members()
    local extra = {
        property("Capacity", "int", "Gets or sets the number of elements that can be contained."),
        property("this[int]", "T", "Gets or sets the element at the specified index."),
        method("Add", "void", "Add(T item)", "Adds an item to the end of the list."),
        method("AddRange", "void", "AddRange(IEnumerable<T> collection)", "Adds a range of elements."),
        method("Insert", "void", "Insert(int index, T item)", "Inserts an element at the specified index."),
        method("InsertRange", "void", "InsertRange(int index, IEnumerable<T> collection)", "Inserts a range of elements."),
        method("Remove", "bool", "Remove(T item)", "Removes the first occurrence of a value."),
        method("RemoveAt", "void", "RemoveAt(int index)", "Removes the element at the specified index."),
        method("RemoveAll", "int", "RemoveAll(Predicate<T> match)", "Removes all elements matching a predicate."),
        method("RemoveRange", "void", "RemoveRange(int index, int count)", "Removes a range of elements."),
        method("IndexOf", "int", "IndexOf(T item)", "Returns the index of the first occurrence."),
        method("LastIndexOf", "int", "LastIndexOf(T item)", "Returns the index of the last occurrence."),
        method("Find", "T", "Find(Predicate<T> match)", "Searches for the first element matching a predicate."),
        method("FindAll", "List<T>", "FindAll(Predicate<T> match)", "Retrieves all elements matching a predicate."),
        method("FindIndex", "int", "FindIndex(Predicate<T> match)", "Searches for an element matching a predicate."),
        method("Exists", "bool", "Exists(Predicate<T> match)", "Determines whether an element matches a predicate."),
        method("Sort", "void", "Sort()", "Sorts the elements."),
        method("Reverse", "void", "Reverse()", "Reverses the order of the elements."),
        method("ForEach", "void", "ForEach(Action<T> action)", "Performs an action on each element."),
        constructor("List<T>()", "Initializes a new instance of List<T>."),
        constructor("List<T>(int capacity)", "Initializes a new instance with a capacity."),
        constructor("List<T>(IEnumerable<T> collection)", "Initializes a new instance from a collection."),
    }
    for _, item in ipairs(extra) do
        result[#result + 1] = item
    end
    return with_object(result)
end

local function dictionary_members()
    return with_object({
        property("Count", "int", "Gets the number of key/value pairs contained in the dictionary.", { readonly = true }),
        property("Keys", "ICollection<TKey>", "Gets the collection of keys.", { readonly = true }),
        property("Values", "ICollection<TValue>", "Gets the collection of values.", { readonly = true }),
        property("this[TKey]", "TValue", "Gets or sets the value associated with a key."),
        method("Add", "void", "Add(TKey key, TValue value)", "Adds a key/value pair."),
        method("Remove", "bool", "Remove(TKey key)", "Removes the value with the specified key."),
        method("Clear", "void", "Clear()", "Removes all keys and values."),
        method("ContainsKey", "bool", "ContainsKey(TKey key)", "Determines whether the dictionary contains a key."),
        method("ContainsValue", "bool", "ContainsValue(TValue value)", "Determines whether the dictionary contains a value."),
        method("TryGetValue", "bool", "TryGetValue(TKey key, out TValue value)", "Gets the value associated with a key."),
        method("TryAdd", "bool", "TryAdd(TKey key, TValue value)", "Attempts to add a key/value pair."),
        method("GetEnumerator", "IEnumerator<KeyValuePair<TKey, TValue>>", "GetEnumerator()", "Returns an enumerator over the dictionary."),
        constructor("Dictionary<TKey, TValue>()", "Initializes a new empty dictionary."),
        constructor("Dictionary<TKey, TValue>(int capacity)", "Initializes a new dictionary with a capacity."),
        constructor("Dictionary<TKey, TValue>(IDictionary<TKey, TValue> dictionary)", "Copies entries from another dictionary."),
    })
end

local function hashset_members()
    return with_object({
        property("Count", "int", "Gets the number of elements.", { readonly = true }),
        method("Add", "bool", "Add(T item)", "Adds an element to the set."),
        method("Remove", "bool", "Remove(T item)", "Removes an element from the set."),
        method("Clear", "void", "Clear()", "Removes all elements."),
        method("Contains", "bool", "Contains(T item)", "Determines whether the set contains a value."),
        method("UnionWith", "void", "UnionWith(IEnumerable<T> other)", "Modifies the set to contain the union."),
        method("IntersectWith", "void", "IntersectWith(IEnumerable<T> other)", "Modifies the set to contain the intersection."),
        method("ExceptWith", "void", "ExceptWith(IEnumerable<T> other)", "Removes elements in another collection."),
        method("IsSubsetOf", "bool", "IsSubsetOf(IEnumerable<T> other)", "Determines whether the set is a subset."),
        method("IsSupersetOf", "bool", "IsSupersetOf(IEnumerable<T> other)", "Determines whether the set is a superset."),
        method("ToArray", "T[]", "ToArray()", "Copies the set to an array."),
        constructor("HashSet<T>()", "Initializes a new empty set."),
    })
end

local function queue_members()
    return with_object({
        property("Count", "int", "Gets the number of elements in the queue.", { readonly = true }),
        method("Enqueue", "void", "Enqueue(T item)", "Adds an item to the end of the queue."),
        method("Dequeue", "T", "Dequeue()", "Removes and returns the item at the beginning."),
        method("Peek", "T", "Peek()", "Returns the item at the beginning without removing it."),
        method("TryDequeue", "bool", "TryDequeue(out T result)", "Attempts to remove and return the item at the beginning."),
        method("TryPeek", "bool", "TryPeek(out T result)", "Attempts to return the item at the beginning."),
        method("Clear", "void", "Clear()", "Removes all elements."),
        method("ToArray", "T[]", "ToArray()", "Copies the queue to an array."),
        constructor("Queue<T>()", "Initializes a new empty queue."),
    })
end

local function stack_members()
    return with_object({
        property("Count", "int", "Gets the number of elements in the stack.", { readonly = true }),
        method("Push", "void", "Push(T item)", "Inserts an item at the top of the stack."),
        method("Pop", "T", "Pop()", "Removes and returns the item at the top."),
        method("Peek", "T", "Peek()", "Returns the item at the top without removing it."),
        method("TryPop", "bool", "TryPop(out T result)", "Attempts to remove and return the item at the top."),
        method("TryPeek", "bool", "TryPeek(out T result)", "Attempts to return the item at the top."),
        method("Clear", "void", "Clear()", "Removes all elements."),
        method("ToArray", "T[]", "ToArray()", "Copies the stack to an array."),
        constructor("Stack<T>()", "Initializes a new empty stack."),
    })
end

local function task_members(generic)
    local members = {
        property("IsCompleted", "bool", "Gets whether the task has completed.", { readonly = true }),
        property("IsFaulted", "bool", "Gets whether the task completed due to an unhandled exception.", { readonly = true }),
        property("IsCanceled", "bool", "Gets whether the task completed due to cancellation.", { readonly = true }),
        property("Exception", "AggregateException", "Gets the aggregate exception that caused the task to end.", { readonly = true }),
        method("Wait", "void", "Wait()", "Waits for the task to complete."),
        method("ContinueWith", "Task", "ContinueWith(Action<Task> continuation)", "Creates a continuation that executes when the task completes."),
        method("ConfigureAwait", "ConfiguredTaskAwaitable", "ConfigureAwait(bool continueOnCapturedContext)", "Configures how awaits on the task behave."),
        method("GetAwaiter", "TaskAwaiter", "GetAwaiter()", "Gets an awaiter used to await this task."),
        static_method("Delay", "Task", "Delay(int millisecondsDelay)", "Creates a task that completes after a time delay."),
        static_method("Run", "Task", "Run(Action action)", "Queues work to run on the thread pool."),
        static_method("WhenAll", "Task", "WhenAll(IEnumerable<Task> tasks)", "Creates a task that completes when all tasks complete."),
        static_method("WhenAny", "Task", "WhenAny(IEnumerable<Task> tasks)", "Creates a task that completes when any task completes."),
        static_method("WaitAll", "void", "WaitAll(params Task[] tasks)", "Waits for all tasks to complete."),
        static_method("WaitAny", "int", "WaitAny(params Task[] tasks)", "Waits for any task to complete."),
        static_property("CompletedTask", "Task", "Gets a successfully completed task."),
        constructor("Task(Action action)", "Initializes a new task."),
    }
    if generic then
        members[#members + 1] = property("Result", "T", "Gets the result value of the task.", { readonly = true })
    end
    return with_object(members)
end

local function stream_members()
    return with_object({
        property("CanRead", "bool", "Gets whether the stream supports reading.", { readonly = true }),
        property("CanWrite", "bool", "Gets whether the stream supports writing.", { readonly = true }),
        property("CanSeek", "bool", "Gets whether the stream supports seeking.", { readonly = true }),
        property("Length", "long", "Gets the length of the stream.", { readonly = true }),
        property("Position", "long", "Gets or sets the position within the stream."),
        method("Read", "int", "Read(byte[] buffer, int offset, int count)", "Reads bytes from the stream."),
        method("ReadAsync", "Task<int>", "ReadAsync(byte[] buffer, int offset, int count)", "Reads bytes asynchronously."),
        method("Write", "void", "Write(byte[] buffer, int offset, int count)", "Writes bytes to the stream."),
        method("WriteAsync", "Task", "WriteAsync(byte[] buffer, int offset, int count)", "Writes bytes asynchronously."),
        method("Flush", "void", "Flush()", "Clears buffers and writes data to the underlying device."),
        method("Seek", "long", "Seek(long offset, SeekOrigin origin)", "Sets the position within the stream."),
        method("Dispose", "void", "Dispose()", "Releases the resources used by the stream."),
    })
end

B.types = {
    ["object"] = {
        name = "object", fullName = "System.Object", namespace = "System",
        kind = "class", members = with_object({}),
        documentation = "The root of the type hierarchy.",
    },
    ["string"] = {
        name = "string", fullName = "System.String", namespace = "System",
        kind = "class", members = with_object({
            property("Length", "int", "Gets the number of characters in the string.", { readonly = true }),
            property("this[int]", "char", "Gets the character at the specified position."),
            static_field("Empty", "string", "Represents the empty string."),
            static_method("IsNullOrEmpty", "bool", "IsNullOrEmpty(string? value)", "Indicates whether a string is null or empty."),
            static_method("IsNullOrWhiteSpace", "bool", "IsNullOrWhiteSpace(string? value)", "Indicates whether a string is null, empty, or whitespace."),
            static_method("Concat", "string", "Concat(params object?[] values)", "Concatenates the string representations of values."),
            static_method("Join", "string", "Join(string? separator, IEnumerable<string?> values)", "Concatenates values with a separator."),
            static_method("Format", "string", "Format(string format, params object?[] args)", "Replaces format items with values."),
            static_method("Compare", "int", "Compare(string? strA, string? strB)", "Compares two strings."),
            method("Contains", "bool", "Contains(string value)", "Returns whether the string contains a value."),
            method("StartsWith", "bool", "StartsWith(string value)", "Determines whether the string starts with a value."),
            method("EndsWith", "bool", "EndsWith(string value)", "Determines whether the string ends with a value."),
            method("IndexOf", "int", "IndexOf(string value)", "Reports the index of the first occurrence of a value."),
            method("LastIndexOf", "int", "LastIndexOf(string value)", "Reports the index of the last occurrence of a value."),
            method("Substring", "string", "Substring(int startIndex)", "Retrieves a substring."),
            method("Replace", "string", "Replace(string oldValue, string? newValue)", "Returns a new string with replacements."),
            method("Remove", "string", "Remove(int startIndex)", "Returns a new string with characters removed."),
            method("Insert", "string", "Insert(int startIndex, string value)", "Returns a new string with a value inserted."),
            method("PadLeft", "string", "PadLeft(int totalWidth)", "Right-aligns characters by padding on the left."),
            method("PadRight", "string", "PadRight(int totalWidth)", "Left-aligns characters by padding on the right."),
            method("Trim", "string", "Trim()", "Removes leading and trailing whitespace."),
            method("TrimStart", "string", "TrimStart()", "Removes leading whitespace."),
            method("TrimEnd", "string", "TrimEnd()", "Removes trailing whitespace."),
            method("ToUpper", "string", "ToUpper()", "Returns a copy converted to uppercase."),
            method("ToLower", "string", "ToLower()", "Returns a copy converted to lowercase."),
            method("ToUpperInvariant", "string", "ToUpperInvariant()", "Returns a copy converted to uppercase using invariant culture."),
            method("ToLowerInvariant", "string", "ToLowerInvariant()", "Returns a copy converted to lowercase using invariant culture."),
            method("Split", "string[]", "Split(char separator)", "Splits a string into substrings."),
            method("ToCharArray", "char[]", "ToCharArray()", "Copies the characters to a character array."),
            method("CompareTo", "int", "CompareTo(string? strB)", "Compares this instance with another string."),
            method("GetEnumerator", "CharEnumerator", "GetEnumerator()", "Returns an enumerator over the characters."),
        }),
        documentation = "Represents immutable text as a sequence of UTF-16 code units.",
    },
    ["int"] = { name = "int", fullName = "System.Int32", namespace = "System", kind = "struct", members = numeric_members("int"), documentation = "A signed 32-bit integer." },
    ["long"] = { name = "long", fullName = "System.Int64", namespace = "System", kind = "struct", members = numeric_members("long"), documentation = "A signed 64-bit integer." },
    ["short"] = { name = "short", fullName = "System.Int16", namespace = "System", kind = "struct", members = numeric_members("short"), documentation = "A signed 16-bit integer." },
    ["byte"] = { name = "byte", fullName = "System.Byte", namespace = "System", kind = "struct", members = numeric_members("byte"), documentation = "An unsigned 8-bit integer." },
    ["sbyte"] = { name = "sbyte", fullName = "System.SByte", namespace = "System", kind = "struct", members = numeric_members("sbyte"), documentation = "A signed 8-bit integer." },
    ["uint"] = { name = "uint", fullName = "System.UInt32", namespace = "System", kind = "struct", members = numeric_members("uint"), documentation = "An unsigned 32-bit integer." },
    ["ulong"] = { name = "ulong", fullName = "System.UInt64", namespace = "System", kind = "struct", members = numeric_members("ulong"), documentation = "An unsigned 64-bit integer." },
    ["ushort"] = { name = "ushort", fullName = "System.UInt16", namespace = "System", kind = "struct", members = numeric_members("ushort"), documentation = "An unsigned 16-bit integer." },
    ["float"] = { name = "float", fullName = "System.Single", namespace = "System", kind = "struct", members = numeric_members("float"), documentation = "A single-precision floating-point number." },
    ["double"] = { name = "double", fullName = "System.Double", namespace = "System", kind = "struct", members = numeric_members("double"), documentation = "A double-precision floating-point number." },
    ["decimal"] = { name = "decimal", fullName = "System.Decimal", namespace = "System", kind = "struct", members = numeric_members("decimal"), documentation = "A decimal floating-point number." },
    ["bool"] = { name = "bool", fullName = "System.Boolean", namespace = "System", kind = "struct", members = with_object({
        static_field("TrueString", "string", "The string representation of true."),
        static_field("FalseString", "string", "The string representation of false."),
        static_method("Parse", "bool", "Parse(string value)", "Converts a string to a Boolean value."),
        static_method("TryParse", "bool", "TryParse(string value, out bool result)", "Attempts to convert a string to a Boolean value."),
    }), documentation = "A Boolean value." },
    ["char"] = { name = "char", fullName = "System.Char", namespace = "System", kind = "struct", members = with_object({
        static_field("MaxValue", "char", "The largest possible character value."),
        static_field("MinValue", "char", "The smallest possible character value."),
        static_method("IsDigit", "bool", "IsDigit(char c)", "Indicates whether a character is a decimal digit."),
        static_method("IsLetter", "bool", "IsLetter(char c)", "Indicates whether a character is a letter."),
        static_method("IsWhiteSpace", "bool", "IsWhiteSpace(char c)", "Indicates whether a character is whitespace."),
        static_method("ToUpper", "char", "ToUpper(char c)", "Converts a character to uppercase."),
        static_method("ToLower", "char", "ToLower(char c)", "Converts a character to lowercase."),
        method("ToString", "string", "ToString()", "Converts the character to a string."),
    }), documentation = "A UTF-16 code unit." },
    ["void"] = { name = "void", fullName = "System.Void", namespace = "System", kind = "struct", members = {}, documentation = "Represents no value." },
    ["dynamic"] = { name = "dynamic", fullName = "System.Object", namespace = "System", kind = "class", members = with_object({}), documentation = "A dynamically typed value." },
    ["nint"] = { name = "nint", fullName = "System.IntPtr", namespace = "System", kind = "struct", members = numeric_members("nint"), documentation = "A native-sized signed integer." },
    ["nuint"] = { name = "nuint", fullName = "System.UIntPtr", namespace = "System", kind = "struct", members = numeric_members("nuint"), documentation = "A native-sized unsigned integer." },
    ["Array"] = { name = "Array", fullName = "System.Array", namespace = "System", kind = "class", members = with_object({
        property("Length", "int", "Gets the total number of elements in all dimensions."),
        property("LongLength", "long", "Gets the total number of elements as a 64-bit integer."),
        property("Rank", "int", "Gets the number of dimensions."),
        static_field("Empty", "T[]", "An empty array."),
        static_method("Resize", "void", "Resize<T>(ref T[] array, int newSize)", "Changes the size of an array."),
        static_method("Sort", "void", "Sort(Array array)", "Sorts the elements of an array."),
        static_method("Reverse", "void", "Reverse(Array array)", "Reverses the sequence of elements."),
        static_method("IndexOf", "int", "IndexOf(Array array, object? value)", "Searches for a value."),
        static_method("BinarySearch", "int", "BinarySearch(Array array, object? value)", "Searches a sorted array."),
        static_method("Copy", "void", "Copy(Array sourceArray, Array destinationArray, int length)", "Copies elements between arrays."),
        static_method("Clear", "void", "Clear(Array array, int index, int length)", "Sets a range of elements to their default values."),
        method("GetLength", "int", "GetLength(int dimension)", "Gets the number of elements in a dimension."),
        method("GetValue", "object?", "GetValue(int index)", "Gets the value at a position."),
        method("SetValue", "void", "SetValue(object? value, int index)", "Sets the value at a position."),
        method("Clone", "object", "Clone()", "Creates a shallow copy."),
    }), documentation = "Provides methods for creating, manipulating, searching, and sorting arrays." },
    ["Console"] = { name = "Console", fullName = "System.Console", namespace = "System", kind = "class", members = with_object({
        static_method("WriteLine", "void", "WriteLine(string? value)", "Writes a value followed by a line terminator."),
        static_method("Write", "void", "Write(string? value)", "Writes a value to the console."),
        static_method("ReadLine", "string?", "ReadLine()", "Reads the next line of characters."),
        static_method("ReadKey", "ConsoleKeyInfo", "ReadKey()", "Obtains the next character or function key."),
        static_method("Read", "int", "Read()", "Reads the next character."),
        static_method("Clear", "void", "Clear()", "Clears the console buffer."),
        static_property("Error", "TextWriter", "Gets the standard error output stream."),
        static_property("Out", "TextWriter", "Gets the standard output stream."),
        static_property("In", "TextReader", "Gets the standard input stream."),
        static_property("ForegroundColor", "ConsoleColor", "Gets or sets the foreground color."),
        static_property("BackgroundColor", "ConsoleColor", "Gets or sets the background color."),
    }), documentation = "Represents the standard input, output, and error streams for console applications." },
    ["Math"] = { name = "Math", fullName = "System.Math", namespace = "System", kind = "class", members = with_object({
        static_field("PI", "double", "The ratio of a circle's circumference to its diameter."),
        static_field("E", "double", "The base of natural logarithms."),
        static_method("Abs", "int", "Abs(int value)", "Returns the absolute value."),
        static_method("Sqrt", "double", "Sqrt(double d)", "Returns the square root."),
        static_method("Pow", "double", "Pow(double x, double y)", "Returns a number raised to a power."),
        static_method("Min", "int", "Min(int val1, int val2)", "Returns the smaller of two values."),
        static_method("Max", "int", "Max(int val1, int val2)", "Returns the larger of two values."),
        static_method("Round", "double", "Round(double a)", "Rounds to the nearest integral value."),
        static_method("Floor", "double", "Floor(double d)", "Returns the largest integral value less than or equal."),
        static_method("Ceiling", "double", "Ceiling(double a)", "Returns the smallest integral value greater than or equal."),
        static_method("Sin", "double", "Sin(double a)", "Returns the sine."),
        static_method("Cos", "double", "Cos(double d)", "Returns the cosine."),
        static_method("Tan", "double", "Tan(double a)", "Returns the tangent."),
        static_method("Log", "double", "Log(double d)", "Returns the natural logarithm."),
        static_method("Exp", "double", "Exp(double d)", "Returns e raised to a power."),
        static_method("Clamp", "int", "Clamp(int value, int min, int max)", "Returns a value clamped to a range."),
        static_method("Sign", "int", "Sign(double value)", "Returns an integer indicating the sign."),
    }), documentation = "Provides constants and static methods for trigonometric, logarithmic, and other common mathematical functions." },
    ["Convert"] = { name = "Convert", fullName = "System.Convert", namespace = "System", kind = "class", members = with_object({
        static_method("ToInt32", "int", "ToInt32(string? value)", "Converts a value to a 32-bit signed integer."),
        static_method("ToInt64", "long", "ToInt64(string? value)", "Converts a value to a 64-bit signed integer."),
        static_method("ToDouble", "double", "ToDouble(string? value)", "Converts a value to a double."),
        static_method("ToBoolean", "bool", "ToBoolean(string? value)", "Converts a value to a Boolean."),
        static_method("ToString", "string", "ToString(object? value)", "Converts a value to a string."),
        static_method("ToDateTime", "DateTime", "ToDateTime(string? value)", "Converts a value to a DateTime."),
        static_method("FromBase64String", "byte[]", "FromBase64String(string s)", "Converts a Base64 string to a byte array."),
        static_method("ToBase64String", "string", "ToBase64String(byte[] inArray)", "Converts a byte array to a Base64 string."),
    }), documentation = "Provides methods for converting base data types." },
    ["DateTime"] = { name = "DateTime", fullName = "System.DateTime", namespace = "System", kind = "struct", members = with_object({
        static_property("Now", "DateTime", "Gets the current local date and time."),
        static_property("UtcNow", "DateTime", "Gets the current UTC date and time."),
        static_property("Today", "DateTime", "Gets the current date."),
        static_property("MinValue", "DateTime", "The smallest possible DateTime value."),
        static_property("MaxValue", "DateTime", "The largest possible DateTime value."),
        property("Year", "int", "Gets the year component."),
        property("Month", "int", "Gets the month component."),
        property("Day", "int", "Gets the day of the month."),
        property("DayOfWeek", "DayOfWeek", "Gets the day of the week."),
        property("DayOfYear", "int", "Gets the day of the year."),
        property("Hour", "int", "Gets the hour component."),
        property("Minute", "int", "Gets the minute component."),
        property("Second", "int", "Gets the second component."),
        property("Millisecond", "int", "Gets the millisecond component."),
        property("Date", "DateTime", "Gets the date component."),
        property("TimeOfDay", "TimeSpan", "Gets the time of day."),
        property("Ticks", "long", "Gets the number of ticks."),
        method("AddDays", "DateTime", "AddDays(double value)", "Returns a new DateTime that adds days."),
        method("AddHours", "DateTime", "AddHours(double value)", "Returns a new DateTime that adds hours."),
        method("AddMinutes", "DateTime", "AddMinutes(double value)", "Returns a new DateTime that adds minutes."),
        method("AddMonths", "DateTime", "AddMonths(int months)", "Returns a new DateTime that adds months."),
        method("AddYears", "DateTime", "AddYears(int years)", "Returns a new DateTime that adds years."),
        method("ToUniversalTime", "DateTime", "ToUniversalTime()", "Converts to UTC."),
        method("ToLocalTime", "DateTime", "ToLocalTime()", "Converts to local time."),
        method("ToString", "string", "ToString(string? format)", "Converts the value to a string."),
        static_method("Parse", "DateTime", "Parse(string s)", "Converts a string to a DateTime."),
        static_method("TryParse", "bool", "TryParse(string? s, out DateTime result)", "Attempts to convert a string to a DateTime."),
    }), documentation = "Represents an instant in time, typically expressed as a date and time of day." },
    ["DateTimeOffset"] = { name = "DateTimeOffset", fullName = "System.DateTimeOffset", namespace = "System", kind = "struct", members = with_object({
        static_property("Now", "DateTimeOffset", "Gets the current date and time with offset."),
        static_property("UtcNow", "DateTimeOffset", "Gets the current UTC date and time."),
        property("DateTime", "DateTime", "Gets the DateTime component."),
        property("Offset", "TimeSpan", "Gets the offset from UTC."),
        method("ToUniversalTime", "DateTimeOffset", "ToUniversalTime()", "Converts to UTC."),
        method("ToLocalTime", "DateTimeOffset", "ToLocalTime()", "Converts to local time."),
        method("ToString", "string", "ToString(string? format)", "Converts the value to a string."),
    }), documentation = "Represents a point in time with an offset from UTC." },
    ["TimeSpan"] = { name = "TimeSpan", fullName = "System.TimeSpan", namespace = "System", kind = "struct", members = with_object({
        static_field("Zero", "TimeSpan", "Represents zero time."),
        static_field("MaxValue", "TimeSpan", "The largest possible TimeSpan."),
        static_field("MinValue", "TimeSpan", "The smallest possible TimeSpan."),
        property("Days", "int", "Gets the days component."),
        property("Hours", "int", "Gets the hours component."),
        property("Minutes", "int", "Gets the minutes component."),
        property("Seconds", "int", "Gets the seconds component."),
        property("Milliseconds", "int", "Gets the milliseconds component."),
        property("TotalDays", "double", "Gets the total number of days."),
        property("TotalHours", "double", "Gets the total number of hours."),
        property("TotalMinutes", "double", "Gets the total number of minutes."),
        property("TotalSeconds", "double", "Gets the total number of seconds."),
        property("TotalMilliseconds", "double", "Gets the total number of milliseconds."),
        static_method("FromDays", "TimeSpan", "FromDays(double value)", "Returns a TimeSpan representing days."),
        static_method("FromHours", "TimeSpan", "FromHours(double value)", "Returns a TimeSpan representing hours."),
        static_method("FromMinutes", "TimeSpan", "FromMinutes(double value)", "Returns a TimeSpan representing minutes."),
        static_method("FromSeconds", "TimeSpan", "FromSeconds(double value)", "Returns a TimeSpan representing seconds."),
        static_method("FromMilliseconds", "TimeSpan", "FromMilliseconds(double value)", "Returns a TimeSpan representing milliseconds."),
        method("ToString", "string", "ToString()", "Converts the value to a string."),
    }), documentation = "Represents a time interval." },
    ["Guid"] = { name = "Guid", fullName = "System.Guid", namespace = "System", kind = "struct", members = with_object({
        static_field("Empty", "Guid", "A Guid with all zero bits."),
        static_method("NewGuid", "Guid", "NewGuid()", "Initializes a new Guid."),
        static_method("Parse", "Guid", "Parse(string input)", "Converts a string to a Guid."),
        static_method("TryParse", "bool", "TryParse(string? input, out Guid result)", "Attempts to convert a string to a Guid."),
        method("ToString", "string", "ToString(string? format)", "Returns the string representation."),
    }), documentation = "Represents a globally unique identifier." },
    ["Random"] = { name = "Random", fullName = "System.Random", namespace = "System", kind = "class", members = with_object({
        static_property("Shared", "Random", "Gets a thread-safe shared Random instance."),
        method("Next", "int", "Next()", "Returns a non-negative random integer."),
        method("Next", "int", "Next(int maxValue)", "Returns a non-negative random integer below a maximum."),
        method("Next", "int", "Next(int minValue, int maxValue)", "Returns a random integer in a range."),
        method("NextDouble", "double", "NextDouble()", "Returns a random floating-point number between 0 and 1."),
        method("NextBytes", "void", "NextBytes(byte[] buffer)", "Fills a byte array with random values."),
        constructor("Random()", "Initializes a new random number generator."),
        constructor("Random(int seed)", "Initializes a new random number generator with a seed."),
    }), documentation = "Represents a pseudo-random number generator." },
    ["Environment"] = { name = "Environment", fullName = "System.Environment", namespace = "System", kind = "class", members = with_object({
        static_property("CurrentDirectory", "string", "Gets or sets the current working directory."),
        static_property("NewLine", "string", "Gets the newline string for this environment."),
        static_property("MachineName", "string", "Gets the NetBIOS name of this computer."),
        static_property("UserName", "string", "Gets the user name of the current user."),
        static_property("OSVersion", "OperatingSystem", "Gets the operating system version."),
        static_property("CommandLine", "string", "Gets the command line for this process."),
        static_method("Exit", "void", "Exit(int exitCode)", "Terminates this process."),
        static_method("GetEnvironmentVariable", "string?", "GetEnvironmentVariable(string variable)", "Retrieves an environment variable."),
        static_method("SetEnvironmentVariable", "void", "SetEnvironmentVariable(string variable, string? value)", "Sets an environment variable."),
        static_method("GetFolderPath", "string", "GetFolderPath(Environment.SpecialFolder folder)", "Gets the path to a special folder."),
    }), documentation = "Provides information about the current environment and platform." },
    ["Uri"] = { name = "Uri", fullName = "System.Uri", namespace = "System", kind = "class", members = with_object({
        property("AbsoluteUri", "string", "Gets the absolute URI."),
        property("Scheme", "string", "Gets the scheme name."),
        property("Host", "string", "Gets the host component."),
        property("Port", "int", "Gets the port number."),
        property("PathAndQuery", "string", "Gets the path and query."),
        property("Query", "string", "Gets the query component."),
        property("IsAbsoluteUri", "bool", "Gets whether the URI is absolute."),
        static_method("TryCreate", "bool", "TryCreate(string? uriString, UriKind uriKind, out Uri? result)", "Creates a Uri if the string is valid."),
        constructor("Uri(string uriString)", "Initializes a new Uri from a string."),
        constructor("Uri(string uriString, UriKind uriKind)", "Initializes a new Uri from a string and kind."),
    }), documentation = "Provides an object representation of a uniform resource identifier." },
    ["List"] = { name = "List", fullName = "System.Collections.Generic.List<T>", namespace = "System.Collections.Generic", kind = "class", members = list_members(), documentation = "Represents a strongly typed list of objects that can be accessed by index.", typeParameters = { "T" } },
    ["Dictionary"] = { name = "Dictionary", fullName = "System.Collections.Generic.Dictionary<TKey, TValue>", namespace = "System.Collections.Generic", kind = "class", members = dictionary_members(), documentation = "Represents a collection of keys and values.", typeParameters = { "TKey", "TValue" } },
    ["HashSet"] = { name = "HashSet", fullName = "System.Collections.Generic.HashSet<T>", namespace = "System.Collections.Generic", kind = "class", members = hashset_members(), documentation = "Represents a set of values.", typeParameters = { "T" } },
    ["Queue"] = { name = "Queue", fullName = "System.Collections.Generic.Queue<T>", namespace = "System.Collections.Generic", kind = "class", members = queue_members(), documentation = "Represents a first-in, first-out collection.", typeParameters = { "T" } },
    ["Stack"] = { name = "Stack", fullName = "System.Collections.Generic.Stack<T>", namespace = "System.Collections.Generic", kind = "class", members = stack_members(), documentation = "Represents a last-in, first-out collection.", typeParameters = { "T" } },
    ["LinkedList"] = { name = "LinkedList", fullName = "System.Collections.Generic.LinkedList<T>", namespace = "System.Collections.Generic", kind = "class", members = with_object({
        property("Count", "int", "Gets the number of nodes."),
        property("First", "LinkedListNode<T>", "Gets the first node."),
        property("Last", "LinkedListNode<T>", "Gets the last node."),
        method("AddFirst", "LinkedListNode<T>", "AddFirst(T value)", "Adds a node at the beginning."),
        method("AddLast", "LinkedListNode<T>", "AddLast(T value)", "Adds a node at the end."),
        method("Remove", "bool", "Remove(T value)", "Removes the first occurrence of a value."),
        method("Clear", "void", "Clear()", "Removes all nodes."),
    }), documentation = "Represents a doubly linked list.", typeParameters = { "T" } },
    ["IEnumerable"] = { name = "IEnumerable", fullName = "System.Collections.Generic.IEnumerable<T>", namespace = "System.Collections.Generic", kind = "interface", members = collection_members(), documentation = "Exposes an enumerator over a collection.", typeParameters = { "T" } },
    ["ICollection"] = { name = "ICollection", fullName = "System.Collections.Generic.ICollection<T>", namespace = "System.Collections.Generic", kind = "interface", members = collection_members(), documentation = "Defines a collection of values.", typeParameters = { "T" } },
    ["IList"] = { name = "IList", fullName = "System.Collections.Generic.IList<T>", namespace = "System.Collections.Generic", kind = "interface", members = with_object({
        property("this[int]", "T", "Gets or sets the element at the specified index."),
        method("IndexOf", "int", "IndexOf(T item)", "Returns the index of an item."),
        method("Insert", "void", "Insert(int index, T item)", "Inserts an item."),
        method("RemoveAt", "void", "RemoveAt(int index)", "Removes the item at an index."),
    }), documentation = "Represents a collection of objects that can be individually accessed by index.", typeParameters = { "T" } },
    ["IReadOnlyList"] = { name = "IReadOnlyList", fullName = "System.Collections.Generic.IReadOnlyList<T>", namespace = "System.Collections.Generic", kind = "interface", members = with_object({
        property("Count", "int", "Gets the number of elements."),
        property("this[int]", "T", "Gets the element at the specified index."),
    }), documentation = "Represents a read-only collection accessed by index.", typeParameters = { "T" } },
    ["IDictionary"] = { name = "IDictionary", fullName = "System.Collections.Generic.IDictionary<TKey, TValue>", namespace = "System.Collections.Generic", kind = "interface", members = dictionary_members(), documentation = "Represents a generic collection of key/value pairs.", typeParameters = { "TKey", "TValue" } },
    ["IReadOnlyDictionary"] = { name = "IReadOnlyDictionary", fullName = "System.Collections.Generic.IReadOnlyDictionary<TKey, TValue>", namespace = "System.Collections.Generic", kind = "interface", members = with_object({
        property("Count", "int", "Gets the number of entries."),
        property("Keys", "IEnumerable<TKey>", "Gets the keys."),
        property("Values", "IEnumerable<TValue>", "Gets the values."),
        property("this[TKey]", "TValue", "Gets the value for a key."),
        method("ContainsKey", "bool", "ContainsKey(TKey key)", "Determines whether a key exists."),
        method("TryGetValue", "bool", "TryGetValue(TKey key, out TValue value)", "Gets the value for a key."),
    }), documentation = "Represents a read-only collection of key/value pairs.", typeParameters = { "TKey", "TValue" } },
    ["KeyValuePair"] = { name = "KeyValuePair", fullName = "System.Collections.Generic.KeyValuePair<TKey, TValue>", namespace = "System.Collections.Generic", kind = "struct", members = with_object({
        property("Key", "TKey", "Gets the key."),
        property("Value", "TValue", "Gets the value."),
    }), documentation = "Defines a key/value pair.", typeParameters = { "TKey", "TValue" } },
    ["Task"] = { name = "Task", fullName = "System.Threading.Tasks.Task", namespace = "System.Threading.Tasks", kind = "class", members = task_members(false), documentation = "Represents an asynchronous operation." },
    ["ValueTask"] = { name = "ValueTask", fullName = "System.Threading.Tasks.ValueTask", namespace = "System.Threading.Tasks", kind = "struct", members = task_members(false), documentation = "Provides a value type that represents an asynchronous operation." },
    ["CancellationToken"] = { name = "CancellationToken", fullName = "System.Threading.CancellationToken", namespace = "System.Threading", kind = "struct", members = with_object({
        static_property("None", "CancellationToken", "Gets an empty token."),
        property("IsCancellationRequested", "bool", "Gets whether cancellation has been requested."),
        property("CanBeCanceled", "bool", "Gets whether the token can be canceled."),
        method("ThrowIfCancellationRequested", "void", "ThrowIfCancellationRequested()", "Throws if cancellation has been requested."),
        method("Register", "CancellationTokenRegistration", "Register(Action callback)", "Registers a cancellation callback."),
    }), documentation = "Propagates notification that operations should be canceled." },
    ["CancellationTokenSource"] = { name = "CancellationTokenSource", fullName = "System.Threading.CancellationTokenSource", namespace = "System.Threading", kind = "class", members = with_object({
        property("Token", "CancellationToken", "Gets the token associated with the source."),
        property("IsCancellationRequested", "bool", "Gets whether cancellation has been requested."),
        method("Cancel", "void", "Cancel()", "Requests cancellation."),
        method("CancelAfter", "void", "CancelAfter(int millisecondsDelay)", "Schedules cancellation after a delay."),
        method("Dispose", "void", "Dispose()", "Releases resources."),
        constructor("CancellationTokenSource()", "Initializes a new cancellation token source."),
    }), documentation = "Signals to a CancellationToken that it should be canceled." },
    ["Exception"] = { name = "Exception", fullName = "System.Exception", namespace = "System", kind = "class", members = with_object({
        property("Message", "string", "Gets a message that describes the current exception."),
        property("StackTrace", "string?", "Gets a string representation of the immediate frames on the call stack."),
        property("InnerException", "Exception?", "Gets the exception that caused the current exception."),
        property("Data", "IDictionary", "Gets a collection of key/value pairs that provide additional user-defined information."),
        property("HResult", "int", "Gets or sets HRESULT, a coded numerical value assigned to a specific exception."),
        property("Source", "string?", "Gets or sets the name of the application or object that causes the error."),
        property("TargetSite", "MethodBase?", "Gets the method that throws the current exception."),
        method("GetBaseException", "Exception", "GetBaseException()", "Returns the root cause of one or more subsequent exceptions."),
        method("ToString", "string", "ToString()", "Creates and returns a string representation of the current exception."),
        constructor("Exception()", "Initializes a new instance of Exception."),
        constructor("Exception(string? message)", "Initializes a new instance with a message."),
        constructor("Exception(string? message, Exception? innerException)", "Initializes a new instance with a message and inner exception."),
    }), documentation = "Represents errors that occur during application execution." },
    ["ArgumentException"] = { name = "ArgumentException", fullName = "System.ArgumentException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when one of the arguments provided to a method is not valid." },
    ["ArgumentNullException"] = { name = "ArgumentNullException", fullName = "System.ArgumentNullException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when a null reference is passed to a method that does not accept it." },
    ["ArgumentOutOfRangeException"] = { name = "ArgumentOutOfRangeException", fullName = "System.ArgumentOutOfRangeException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when an argument is outside the allowable range of values." },
    ["InvalidOperationException"] = { name = "InvalidOperationException", fullName = "System.InvalidOperationException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when a method call is invalid for the object's current state." },
    ["NotImplementedException"] = { name = "NotImplementedException", fullName = "System.NotImplementedException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when a requested method or operation is not implemented." },
    ["NotSupportedException"] = { name = "NotSupportedException", fullName = "System.NotSupportedException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when an invoked method is not supported." },
    ["NullReferenceException"] = { name = "NullReferenceException", fullName = "System.NullReferenceException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when there is an attempt to dereference a null object reference." },
    ["ObjectDisposedException"] = { name = "ObjectDisposedException", fullName = "System.ObjectDisposedException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when an operation is performed on a disposed object." },
    ["TimeoutException"] = { name = "TimeoutException", fullName = "System.TimeoutException", namespace = "System", kind = "class", members = with_object({}), documentation = "The exception thrown when the time allotted for a process or operation has expired." },
    ["AggregateException"] = { name = "AggregateException", fullName = "System.AggregateException", namespace = "System", kind = "class", members = with_object({}), documentation = "Represents one or more errors that occur during application execution." },
    ["StringBuilder"] = { name = "StringBuilder", fullName = "System.Text.StringBuilder", namespace = "System.Text", kind = "class", members = with_object({
        property("Length", "int", "Gets or sets the length of the builder."),
        property("Capacity", "int", "Gets or sets the maximum number of characters."),
        property("this[int]", "char", "Gets or sets the character at a position."),
        method("Append", "StringBuilder", "Append(string? value)", "Appends a string."),
        method("AppendLine", "StringBuilder", "AppendLine(string? value)", "Appends a string followed by a line terminator."),
        method("AppendFormat", "StringBuilder", "AppendFormat(string format, params object?[] args)", "Appends a formatted string."),
        method("Insert", "StringBuilder", "Insert(int index, string? value)", "Inserts a string."),
        method("Remove", "StringBuilder", "Remove(int startIndex, int length)", "Removes characters."),
        method("Replace", "StringBuilder", "Replace(string oldValue, string? newValue)", "Replaces occurrences."),
        method("Clear", "StringBuilder", "Clear()", "Removes all characters."),
        method("ToString", "string", "ToString()", "Converts the builder to a string."),
        constructor("StringBuilder()", "Initializes a new StringBuilder."),
        constructor("StringBuilder(string? value)", "Initializes a new StringBuilder with a value."),
        constructor("StringBuilder(int capacity)", "Initializes a new StringBuilder with a capacity."),
    }), documentation = "Represents a mutable string of characters." },
    ["Encoding"] = { name = "Encoding", fullName = "System.Text.Encoding", namespace = "System.Text", kind = "class", members = with_object({
        static_property("UTF8", "Encoding", "Gets UTF-8 encoding."),
        static_property("UTF32", "Encoding", "Gets UTF-32 encoding."),
        static_property("ASCII", "Encoding", "Gets ASCII encoding."),
        static_property("Unicode", "Encoding", "Gets UTF-16 encoding."),
        method("GetBytes", "byte[]", "GetBytes(string s)", "Encodes a string to bytes."),
        method("GetString", "string", "GetString(byte[] bytes)", "Decodes bytes to a string."),
        method("GetByteCount", "int", "GetByteCount(string s)", "Calculates the number of bytes."),
        method("GetCharCount", "int", "GetCharCount(byte[] bytes)", "Calculates the number of characters."),
    }), documentation = "Represents a character encoding." },
    ["JsonSerializer"] = { name = "JsonSerializer", fullName = "System.Text.Json.JsonSerializer", namespace = "System.Text.Json", kind = "class", members = with_object({
        static_method("Serialize", "string", "Serialize<TValue>(TValue value)", "Converts a value to a JSON string."),
        static_method("Deserialize", "TValue?", "Deserialize<TValue>(string json)", "Parses JSON into a value."),
        static_method("SerializeAsync", "Task<string>", "SerializeAsync<TValue>(Stream utf8Json, TValue value)", "Serializes a value to JSON asynchronously."),
        static_method("DeserializeAsync", "ValueTask<TValue?>", "DeserializeAsync<TValue>(Stream utf8Json)", "Deserializes JSON asynchronously."),
    }), documentation = "Provides functionality to serialize objects or value types to JSON and deserialize JSON into objects or value types." },
    ["JsonDocument"] = { name = "JsonDocument", fullName = "System.Text.Json.JsonDocument", namespace = "System.Text.Json", kind = "class", members = with_object({
        property("RootElement", "JsonElement", "Gets the root element."),
        static_method("Parse", "JsonDocument", "Parse(string json)", "Parses a JSON string into a document."),
        method("Dispose", "void", "Dispose()", "Releases resources."),
    }), documentation = "Provides a mechanism for examining the structural content of a JSON value." },
    ["JsonElement"] = { name = "JsonElement", fullName = "System.Text.Json.JsonElement", namespace = "System.Text.Json", kind = "struct", members = with_object({
        property("ValueKind", "JsonValueKind", "Gets the kind of the element."),
        method("GetString", "string", "GetString()", "Gets the value as a string."),
        method("GetInt32", "int", "GetInt32()", "Gets the value as an integer."),
        method("GetBoolean", "bool", "GetBoolean()", "Gets the value as a Boolean."),
        method("GetProperty", "JsonElement", "GetProperty(string propertyName)", "Gets a property by name."),
        method("TryGetProperty", "bool", "TryGetProperty(string propertyName, out JsonElement value)", "Attempts to get a property by name."),
        method("EnumerateArray", "JsonElement.ArrayEnumerator", "EnumerateArray()", "Enumerates the array."),
        method("EnumerateObject", "JsonElement.ObjectEnumerator", "EnumerateObject()", "Enumerates the object properties."),
    }), documentation = "Represents a specific JSON value within a JsonDocument." },
    ["Regex"] = { name = "Regex", fullName = "System.Text.RegularExpressions.Regex", namespace = "System.Text.RegularExpressions", kind = "class", members = with_object({
        static_method("IsMatch", "bool", "IsMatch(string input, string pattern)", "Indicates whether the pattern matches in the input."),
        static_method("Match", "Match", "Match(string input, string pattern)", "Searches for the first match."),
        static_method("Matches", "MatchCollection", "Matches(string input, string pattern)", "Searches for all matches."),
        static_method("Replace", "string", "Replace(string input, string pattern, string replacement)", "Replaces matches."),
        static_method("Split", "string[]", "Split(string input, string pattern)", "Splits the input at matches."),
        method("IsMatch", "bool", "IsMatch(string input)", "Indicates whether the pattern matches in the input."),
        method("Match", "Match", "Match(string input)", "Searches for the first match."),
        method("Matches", "MatchCollection", "Matches(string input)", "Searches for all matches."),
        constructor("Regex(string pattern)", "Initializes a new Regex."),
    }), documentation = "Represents an immutable regular expression." },
    ["File"] = { name = "File", fullName = "System.IO.File", namespace = "System.IO", kind = "class", members = with_object({
        static_method("ReadAllText", "string", "ReadAllText(string path)", "Opens a text file and reads all text."),
        static_method("ReadAllLines", "string[]", "ReadAllLines(string path)", "Reads all lines of a file."),
        static_method("ReadAllBytes", "byte[]", "ReadAllBytes(string path)", "Reads all bytes of a file."),
        static_method("WriteAllText", "void", "WriteAllText(string path, string? contents)", "Creates or overwrites a file with text."),
        static_method("WriteAllLines", "void", "WriteAllLines(string path, IEnumerable<string> contents)", "Writes lines to a file."),
        static_method("WriteAllBytes", "void", "WriteAllBytes(string path, byte[] bytes)", "Writes bytes to a file."),
        static_method("AppendAllText", "void", "AppendAllText(string path, string? contents)", "Appends text to a file."),
        static_method("Exists", "bool", "Exists(string? path)", "Determines whether a file exists."),
        static_method("Delete", "void", "Delete(string path)", "Deletes a file."),
        static_method("Copy", "void", "Copy(string sourceFileName, string destFileName)", "Copies a file."),
        static_method("Move", "void", "Move(string sourceFileName, string destFileName)", "Moves a file."),
        static_method("OpenRead", "FileStream", "OpenRead(string path)", "Opens a file for reading."),
        static_method("OpenWrite", "FileStream", "OpenWrite(string path)", "Opens a file for writing."),
        static_method("Create", "FileStream", "Create(string path)", "Creates or overwrites a file."),
    }), documentation = "Provides static methods for creating, copying, deleting, moving, and opening files." },
    ["Directory"] = { name = "Directory", fullName = "System.IO.Directory", namespace = "System.IO", kind = "class", members = with_object({
        static_method("Exists", "bool", "Exists(string? path)", "Determines whether a directory exists."),
        static_method("CreateDirectory", "DirectoryInfo", "CreateDirectory(string path)", "Creates a directory."),
        static_method("Delete", "void", "Delete(string path)", "Deletes a directory."),
        static_method("GetFiles", "string[]", "GetFiles(string path)", "Returns the names of files in a directory."),
        static_method("GetDirectories", "string[]", "GetDirectories(string path)", "Returns the names of subdirectories."),
        static_method("EnumerateFiles", "IEnumerable<string>", "EnumerateFiles(string path)", "Enumerates the files in a directory."),
        static_method("EnumerateDirectories", "IEnumerable<string>", "EnumerateDirectories(string path)", "Enumerates the subdirectories."),
        static_method("GetCurrentDirectory", "string", "GetCurrentDirectory()", "Gets the current working directory."),
        static_method("SetCurrentDirectory", "void", "SetCurrentDirectory(string path)", "Sets the current working directory."),
        static_method("Move", "void", "Move(string sourceDirName, string destDirName)", "Moves a directory."),
    }), documentation = "Exposes static methods for creating, moving, and enumerating through directories and subdirectories." },
    ["Path"] = { name = "Path", fullName = "System.IO.Path", namespace = "System.IO", kind = "class", members = with_object({
        static_field("DirectorySeparatorChar", "char", "The platform directory separator character."),
        static_field("AltDirectorySeparatorChar", "char", "The alternate directory separator character."),
        static_field("PathSeparator", "char", "The platform path separator character."),
        static_method("Combine", "string", "Combine(params string[] paths)", "Combines path strings."),
        static_method("GetFullPath", "string", "GetFullPath(string path)", "Gets the absolute path."),
        static_method("GetFileName", "string?", "GetFileName(string? path)", "Gets the file name and extension."),
        static_method("GetFileNameWithoutExtension", "string?", "GetFileNameWithoutExtension(string? path)", "Gets the file name without extension."),
        static_method("GetExtension", "string?", "GetExtension(string? path)", "Gets the file extension."),
        static_method("GetDirectoryName", "string?", "GetDirectoryName(string? path)", "Gets the directory information."),
        static_method("GetTempPath", "string", "GetTempPath()", "Gets the temporary directory path."),
        static_method("GetTempFileName", "string", "GetTempFileName()", "Creates a uniquely named temporary file."),
        static_method("ChangeExtension", "string?", "ChangeExtension(string? path, string? extension)", "Changes the extension of a path."),
        static_method("HasExtension", "bool", "HasExtension(string? path)", "Determines whether a path includes an extension."),
        static_method("IsPathRooted", "bool", "IsPathRooted(string? path)", "Determines whether a path is rooted."),
    }), documentation = "Performs operations on strings that contain file or directory path information." },
    ["FileStream"] = { name = "FileStream", fullName = "System.IO.FileStream", namespace = "System.IO", kind = "class", members = stream_members(), documentation = "Provides a Stream for a file, supporting both synchronous and asynchronous read and write operations." },
    ["Stream"] = { name = "Stream", fullName = "System.IO.Stream", namespace = "System.IO", kind = "class", members = stream_members(), documentation = "Provides a generic view of a sequence of bytes." },
    ["StreamReader"] = { name = "StreamReader", fullName = "System.IO.StreamReader", namespace = "System.IO", kind = "class", members = with_object({
        method("ReadLine", "string?", "ReadLine()", "Reads a line of characters."),
        method("ReadToEnd", "string", "ReadToEnd()", "Reads all characters to the end."),
        method("Read", "int", "Read()", "Reads the next character."),
        method("Dispose", "void", "Dispose()", "Releases resources."),
        constructor("StreamReader(Stream stream)", "Initializes a new StreamReader."),
        constructor("StreamReader(string path)", "Initializes a new StreamReader for a file."),
    }), documentation = "Implements a TextReader that reads characters from a byte stream." },
    ["StreamWriter"] = { name = "StreamWriter", fullName = "System.IO.StreamWriter", namespace = "System.IO", kind = "class", members = with_object({
        method("Write", "void", "Write(string? value)", "Writes a string."),
        method("WriteLine", "void", "WriteLine(string? value)", "Writes a string followed by a line terminator."),
        method("Flush", "void", "Flush()", "Clears buffers."),
        method("Dispose", "void", "Dispose()", "Releases resources."),
        constructor("StreamWriter(Stream stream)", "Initializes a new StreamWriter."),
        constructor("StreamWriter(string path)", "Initializes a new StreamWriter for a file."),
    }), documentation = "Implements a TextWriter for writing characters to a stream." },
    ["HttpClient"] = { name = "HttpClient", fullName = "System.Net.Http.HttpClient", namespace = "System.Net.Http", kind = "class", members = with_object({
        method("GetStringAsync", "Task<string>", "GetStringAsync(string? requestUri)", "Sends a GET request and returns the body as a string."),
        method("GetAsync", "Task<HttpResponseMessage>", "GetAsync(string? requestUri)", "Sends a GET request."),
        method("PostAsync", "Task<HttpResponseMessage>", "PostAsync(string? requestUri, HttpContent? content)", "Sends a POST request."),
        method("PutAsync", "Task<HttpResponseMessage>", "PutAsync(string? requestUri, HttpContent? content)", "Sends a PUT request."),
        method("DeleteAsync", "Task<HttpResponseMessage>", "DeleteAsync(string? requestUri)", "Sends a DELETE request."),
        method("SendAsync", "Task<HttpResponseMessage>", "SendAsync(HttpRequestMessage request)", "Sends an HTTP request."),
        method("Dispose", "void", "Dispose()", "Releases resources."),
        constructor("HttpClient()", "Initializes a new HttpClient."),
    }), documentation = "Provides a class for sending HTTP requests and receiving HTTP responses." },
    ["HttpResponseMessage"] = { name = "HttpResponseMessage", fullName = "System.Net.Http.HttpResponseMessage", namespace = "System.Net.Http", kind = "class", members = with_object({
        property("StatusCode", "HttpStatusCode", "Gets or sets the status code."),
        property("IsSuccessStatusCode", "bool", "Gets whether the response was successful."),
        property("Content", "HttpContent?", "Gets or sets the response content."),
        method("EnsureSuccessStatusCode", "HttpResponseMessage", "EnsureSuccessStatusCode()", "Throws if the status code is not successful."),
        method("Dispose", "void", "Dispose()", "Releases resources."),
    }), documentation = "Represents an HTTP response message." },
    ["Type"] = { name = "Type", fullName = "System.Type", namespace = "System", kind = "class", members = with_object({
        property("Name", "string", "Gets the name of the type."),
        property("FullName", "string?", "Gets the fully qualified name of the type."),
        property("Namespace", "string?", "Gets the namespace of the type."),
        property("IsClass", "bool", "Gets whether the type is a class."),
        property("IsInterface", "bool", "Gets whether the type is an interface."),
        property("IsEnum", "bool", "Gets whether the type is an enum."),
        property("IsValueType", "bool", "Gets whether the type is a value type."),
        property("IsAbstract", "bool", "Gets whether the type is abstract."),
        property("IsSealed", "bool", "Gets whether the type is sealed."),
        property("BaseType", "Type?", "Gets the base type."),
        property("Assembly", "Assembly", "Gets the assembly of the type."),
        static_method("GetType", "Type?", "GetType(string typeName)", "Gets a Type by name."),
        method("GetMethod", "MethodInfo?", "GetMethod(string name)", "Gets a method by name."),
        method("GetProperty", "PropertyInfo?", "GetProperty(string name)", "Gets a property by name."),
        method("GetField", "FieldInfo?", "GetField(string name)", "Gets a field by name."),
        method("GetMembers", "MemberInfo[]", "GetMembers()", "Gets all members."),
        method("GetCustomAttributes", "object[]", "GetCustomAttributes(bool inherit)", "Gets custom attributes."),
        method("IsAssignableFrom", "bool", "IsAssignableFrom(Type? c)", "Determines whether an instance can be assigned to this type."),
    }), documentation = "Represents type declarations for classes, interfaces, arrays, value types, enumerations, delegates, and built-in types." },
    ["Delegate"] = { name = "Delegate", fullName = "System.Delegate", namespace = "System", kind = "class", members = with_object({
        method("Invoke", "object?", "Invoke(params object?[] args)", "Invokes the delegate."),
        method("BeginInvoke", "IAsyncResult", "BeginInvoke(AsyncCallback? callback, object? state)", "Begins an asynchronous invocation."),
        method("EndInvoke", "object?", "EndInvoke(IAsyncResult result)", "Ends an asynchronous invocation."),
        static_method("Combine", "Delegate", "Combine(Delegate? a, Delegate? b)", "Combines delegates."),
        static_method("Remove", "Delegate?", "Remove(Delegate? source, Delegate? value)", "Removes a delegate."),
    }), documentation = "Represents a delegate, which is a data structure that refers to a static method or an instance method." },
    ["Action"] = { name = "Action", fullName = "System.Action", namespace = "System", kind = "class", members = with_object({
        method("Invoke", "void", "Invoke()", "Invokes the delegate."),
        constructor("Action(object? target, IntPtr method)", "Initializes a new Action."),
    }), documentation = "Encapsulates a method that has no parameters and does not return a value." },
    ["Func"] = { name = "Func", fullName = "System.Func<TResult>", namespace = "System", kind = "class", members = with_object({
        method("Invoke", "TResult", "Invoke()", "Invokes the delegate."),
        constructor("Func(object? target, IntPtr method)", "Initializes a new Func."),
    }), documentation = "Encapsulates a method that has no parameters and returns a value." },
    ["Nullable"] = { name = "Nullable", fullName = "System.Nullable<T>", namespace = "System", kind = "struct", members = with_object({
        property("HasValue", "bool", "Gets whether the value is not null."),
        property("Value", "T", "Gets the underlying value."),
        method("GetValueOrDefault", "T", "GetValueOrDefault()", "Gets the value or a default."),
    }), documentation = "Represents a value type that can be assigned null.", typeParameters = { "T" } },
    ["IEnumerator"] = { name = "IEnumerator", fullName = "System.Collections.IEnumerator", namespace = "System.Collections", kind = "interface", members = with_object({
        property("Current", "object?", "Gets the current element."),
        method("MoveNext", "bool", "MoveNext()", "Advances to the next element."),
        method("Reset", "void", "Reset()", "Sets the enumerator to its initial position."),
    }), documentation = "Supports a simple iteration over a non-generic collection." },
    ["IDisposable"] = { name = "IDisposable", fullName = "System.IDisposable", namespace = "System", kind = "interface", members = with_object({
        method("Dispose", "void", "Dispose()", "Releases unmanaged resources."),
    }), documentation = "Provides a mechanism for releasing unmanaged resources." },
    ["IAsyncDisposable"] = { name = "IAsyncDisposable", fullName = "System.IAsyncDisposable", namespace = "System", kind = "interface", members = with_object({
        method("DisposeAsync", "ValueTask", "DisposeAsync()", "Releases resources asynchronously."),
    }), documentation = "Provides a mechanism for releasing unmanaged resources asynchronously." },
    ["IComparable"] = { name = "IComparable", fullName = "System.IComparable", namespace = "System", kind = "interface", members = with_object({
        method("CompareTo", "int", "CompareTo(object? obj)", "Compares the current instance with another object."),
    }), documentation = "Defines a generalized comparison method." },
    ["IEquatable"] = { name = "IEquatable", fullName = "System.IEquatable<T>", namespace = "System", kind = "interface", members = with_object({
        method("Equals", "bool", "Equals(T? other)", "Indicates whether the current object is equal to another object."),
    }), documentation = "Defines a generalized method for determining equality.", typeParameters = { "T" } },
    ["ICloneable"] = { name = "ICloneable", fullName = "System.ICloneable", namespace = "System", kind = "interface", members = with_object({
        method("Clone", "object", "Clone()", "Creates a new object that is a copy of the current instance."),
    }), documentation = "Supports cloning." },
    ["Span"] = { name = "Span", fullName = "System.Span<T>", namespace = "System", kind = "struct", members = with_object({
        property("Length", "int", "Gets the number of elements."),
        property("this[int]", "T", "Gets or sets the element at an index."),
        method("Slice", "Span<T>", "Slice(int start)", "Forms a slice out of the span."),
        method("ToArray", "T[]", "ToArray()", "Copies the span to an array."),
        static_property("Empty", "Span<T>", "Gets an empty span."),
    }), documentation = "Provides a type-safe and memory-safe representation of a contiguous region of arbitrary memory.", typeParameters = { "T" } },
    ["Memory"] = { name = "Memory", fullName = "System.Memory<T>", namespace = "System", kind = "struct", members = with_object({
        property("Length", "int", "Gets the number of elements."),
        property("this[int]", "T", "Gets or sets the element at an index."),
        method("Slice", "Memory<T>", "Slice(int start)", "Forms a slice out of the memory."),
        method("ToArray", "T[]", "ToArray()", "Copies the memory to an array."),
        static_property("Empty", "Memory<T>", "Gets an empty memory."),
    }), documentation = "Represents a contiguous region of memory.", typeParameters = { "T" } },
    ["ILogger"] = { name = "ILogger", fullName = "Microsoft.Extensions.Logging.ILogger", namespace = "Microsoft.Extensions.Logging", kind = "interface", members = with_object({
        method("Log", "void", "Log(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)", "Writes a log entry."),
        method("IsEnabled", "bool", "IsEnabled(LogLevel logLevel)", "Checks whether logging is enabled."),
        method("BeginScope", "IDisposable?", "BeginScope<TState>(TState state)", "Begins a logical operation scope."),
    }), documentation = "Represents a logger." },
    ["ILoggerFactory"] = { name = "ILoggerFactory", fullName = "Microsoft.Extensions.Logging.ILoggerFactory", namespace = "Microsoft.Extensions.Logging", kind = "interface", members = with_object({
        method("CreateLogger", "ILogger", "CreateLogger(string categoryName)", "Creates a logger."),
        method("AddProvider", "void", "AddProvider(ILoggerProvider provider)", "Adds a logger provider."),
        method("Dispose", "void", "Dispose()", "Releases resources."),
    }), documentation = "Represents a type used to configure the logging system." },
    ["IServiceProvider"] = { name = "IServiceProvider", fullName = "System.IServiceProvider", namespace = "System", kind = "interface", members = with_object({
        method("GetService", "object?", "GetService(Type serviceType)", "Gets the service object of the specified type."),
    }), documentation = "Provides a mechanism for retrieving a service object." },
    ["IServiceCollection"] = { name = "IServiceCollection", fullName = "Microsoft.Extensions.DependencyInjection.IServiceCollection", namespace = "Microsoft.Extensions.DependencyInjection", kind = "interface", members = with_object({
        method("AddSingleton", "IServiceCollection", "AddSingleton(Type serviceType, Type implementationType)", "Adds a singleton service."),
        method("AddScoped", "IServiceCollection", "AddScoped(Type serviceType, Type implementationType)", "Adds a scoped service."),
        method("AddTransient", "IServiceCollection", "AddTransient(Type serviceType, Type implementationType)", "Adds a transient service."),
        method("BuildServiceProvider", "IServiceProvider", "BuildServiceProvider()", "Builds a service provider."),
    }), documentation = "Represents a collection of service descriptors." },
    ["Debug"] = { name = "Debug", fullName = "System.Diagnostics.Debug", namespace = "System.Diagnostics", kind = "class", members = with_object({
        static_method("WriteLine", "void", "WriteLine(string? message)", "Writes a message to the debug output."),
        static_method("Write", "void", "Write(string? message)", "Writes a message to the debug output."),
        static_method("Assert", "void", "Assert(bool condition)", "Checks a condition."),
        static_method("Fail", "void", "Fail(string? message)", "Emits an error message."),
    }), documentation = "Provides a set of methods and properties that help debug your code." },
    ["Trace"] = { name = "Trace", fullName = "System.Diagnostics.Trace", namespace = "System.Diagnostics", kind = "class", members = with_object({
        static_method("WriteLine", "void", "WriteLine(string? message)", "Writes a message to the trace output."),
        static_method("Write", "void", "Write(string? message)", "Writes a message to the trace output."),
        static_method("TraceInformation", "void", "TraceInformation(string? message)", "Writes an informational message."),
    }), documentation = "Provides a set of methods and properties that help you trace the execution of your code." },
}

for _, name in ipairs({ "int", "long", "short", "byte", "sbyte", "uint", "ulong",
    "ushort", "float", "double", "decimal", "bool", "char", "string", "object" }) do
    local item = B.types[name]
    if item then
        item.alias = true
    end
end
B.types["Int32"] = B.types["int"]
B.types["Int64"] = B.types["long"]
B.types["Int16"] = B.types["short"]
B.types["Byte"] = B.types["byte"]
B.types["SByte"] = B.types["sbyte"]
B.types["UInt32"] = B.types["uint"]
B.types["UInt64"] = B.types["ulong"]
B.types["UInt16"] = B.types["ushort"]
B.types["Single"] = B.types["float"]
B.types["Double"] = B.types["double"]
B.types["Decimal"] = B.types["decimal"]
B.types["Boolean"] = B.types["bool"]
B.types["Char"] = B.types["char"]
B.types["String"] = B.types["string"]
B.types["Object"] = B.types["object"]
B.types["Void"] = B.types["void"]
B.types["IntPtr"] = B.types["nint"]
B.types["UIntPtr"] = B.types["nuint"]

B.alias_full = {
    ["bool"] = "System.Boolean", ["byte"] = "System.Byte",
    ["char"] = "System.Char", ["decimal"] = "System.Decimal",
    ["double"] = "System.Double", ["float"] = "System.Single",
    ["int"] = "System.Int32", ["long"] = "System.Int64",
    ["object"] = "System.Object", ["sbyte"] = "System.SByte",
    ["short"] = "System.Int16", ["string"] = "System.String",
    ["uint"] = "System.UInt32", ["ulong"] = "System.UInt64",
    ["ushort"] = "System.UInt16", ["void"] = "System.Void",
    ["dynamic"] = "System.Object", ["nint"] = "System.IntPtr",
    ["nuint"] = "System.UIntPtr",
}

function B.type(name)
    if not name then
        return nil
    end
    name = name:gsub("^@", "")
    local simple = name:match("^([%w_]+)")
    local value = B.types[name] or B.types[simple]
    if value then
        return value
    end
    for _, item in pairs(B.types) do
        if item.name == name or item.fullName == name or
            (item.fullName and item.fullName:match("<") and
                item.fullName:gsub("<.*$", "") == name:gsub("<.*$", "")) then
            return item
        end
    end
    return nil
end

function B.members(type_name)
    local value = B.type(type_name)
    return value and value.members or {}
end

function B.namespace_types(namespace)
    local result = {}
    local seen = {}
    for _, item in pairs(B.types) do
        if item.namespace == namespace and item.name and not seen[item] then
            seen[item] = true
            result[#result + 1] = item
        end
    end
    table.sort(result, function(left, right)
        return (left.name or "") < (right.name or "")
    end)
    return result
end

function B.all_types()
    local seen = {}
    local result = {}
    for _, item in pairs(B.types) do
        if not seen[item] then
            seen[item] = true
            result[#result + 1] = item
        end
    end
    table.sort(result, function(left, right)
        return (left.name or "") < (right.name or "")
    end)
    return result
end

B.enumerable_members = enumerable_members
B.enum_members = enum_members

return B
