; Z80 half of P30: a loop reading Z80 RAM, counting its iterations in $1F00 (16 bits).
    .org 0
    di
    ld sp,$1FF0
    ld de,0
loop:
    ld a,($0100)
    inc de
    ld ($1F00),de
    jr loop
