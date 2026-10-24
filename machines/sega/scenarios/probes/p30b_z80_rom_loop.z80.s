; Z80 half of P30: a loop reading the 68000 ROM through the bank window, counting its iterations in $1F00 (16 bits).
    .org 0
    di
    ld sp,$1FF0
    ld de,0
loop:
    ld a,($8000)
    inc de
    ld ($1F00),de
    jr loop
