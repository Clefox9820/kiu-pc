# The USA release's address of every European address the overrides in this
# folder name in the game's code, found by matching their code with
# scripts\find-usa-addresses.ps1, never by guessing an offset.
# scripts\setup.ps1 rewrites a copy of the overrides with it for the USA
# build, and stops when an override names a code address missing here.
# 3dsrecomp reads only the *.c files in this folder.
@{
    "0x006EA18C" = "0x006EA040"
    "0x00A3F050" = "0x00A3EDCC"
    "0x0074643C" = "0x007462A4"
    "0x006D27E8" = "0x006D269C"
    "0x006D2C08" = "0x006D2ABC"
    "0x00A426AC" = "0x00A42428"
    "0x0070419C" = "0x00704004"
    "0x00606534" = "0x006063E8"
    "0x0039025C" = "0x003901C4"
    "0x003902B0" = "0x00390218"
}
