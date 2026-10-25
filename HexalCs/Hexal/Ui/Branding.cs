namespace Hexal.Ui;

/// <summary>
/// Text shown by the <c>:about</c> command. The branding is embedded as a C#
/// literal so that published single-file builds never need the original text
/// file at runtime.
/// </summary>
internal static class Branding
{
    public const string Title = $"Hexal - {Program.Version}";
    public const string Author = "Author: Navid Momtahen (C) 2026";
    public const string Commit = $"Commit: {BuildInfo.CommitHash}";

    public const string Banner = """
                                           ====
                                       .=== -- -==.
                                    :==-    --    -==-
                                 ===:       --       :===
                              ===.          ==           ===
                           ===           ===  ===           ===
                       .===           ===        ===.          ===.
                    :==-          :==-              ===:          ===-
                 ===-          -==-                    -==-          :==-
              ===.          ===:                          .===          .===
           ===           ===. =                               ===           ===
          ===         ===     =              .                   ===.        ===
          =  ===. .===        =           -==-                      -==: .===  =
          =     -=-          ==        -==: --                         -=-     =
          =      =       .===:=       =:    --                          =      =
          =      =     -==    =       =     -=========-     -========-  =      =
          =      =     -=     =       =     -===.     =-   ==     :=.   =      =
          =      =     -=     =       =     --  =.     == ==     :=     =      =
          =      =     -=     =       =     --   =:     ==-     =+      =      =
          =      =     -=     =       =     --    ==     .     ==       =      =
          =      =     -=     =========     --     ==         +-        =      =
          =      =     -=                   --      -=       =-         =      =
          =      =     -=                   --      -=       =-         =      =
          =      =     -=     =========     --     ==         =-        =      =
          =      =     -=     =       =     -=    ==           ==       =      =
          =      =     -=     =       =     --   +:     +=-     =+      =      =
          =      =     -=     =       =     =- .=.     == ==     :=     =      =
          =      =     -=     =       =     -===.     +-   ==     :=.   =      =
          =      =     -==    =       =    -==========-     -========-  =      =
          =      =       .===:=       =.-==-                            =      =
          =     -=-          ==       ==.                              -=-     =
          =  -==. :==-        =                                     -==: .===  =
          ===        .===     =          ===                     ===.        ===
           ===           ===  =             ==                ===           ===
              ===.          ===.          ==-             .===          .===
                 ===:          -==:      =    .====.   .==-          :===
                    -==-          :==-              -==-          -==-
                       .===          .===        ===:          -==:
                           ===           ===  ===           ===.
                              ===           ==           ===
                                 ===:       --       .===
                                    -==-    --    :===
                                       :==- -- -==:
                                          .====.
        """;
}
