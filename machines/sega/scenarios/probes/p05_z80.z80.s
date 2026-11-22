; The Z80 half of P05: writes its observations to Z80 RAM $1F00-$1F3F, the 68000 collects them.
    .org 0
    di
    ld sp,$1FF0
    ld a,$AA
    ld ($1F00),a
    ld a,$55
    ld ($3F01),a
    ld a,($1F01)
    ld ($1F10),a
    ld a,($3F00)
    ld ($1F11),a
    ld a,($4000)
    ld ($1F12),a
    ld a,($4001)
    ld ($1F13),a
    ld a,($4002)
    ld ($1F14),a
    ld a,($4003)
    ld ($1F15),a
    ld a,($5FFF)
    ld ($1F16),a
    ld a,($4004)
    ld ($1F17),a
    xor a
    ld b,9
bank0:
    ld ($6000),a
    djnz bank0
    ld a,($8100)
    ld ($1F20),a
    ld a,($8101)
    ld ($1F21),a
    ld a,1
    ld b,9
bank1:
    ld ($6000),a
    djnz bank1
    ld a,($8000)
    ld ($1F22),a
    ld a,($8001)
    ld ($1F23),a
    ld a,($6000)
    ld ($1F24),a
    ld a,($7F04)
    ld ($1F28),a
    ld a,($7F05)
    ld ($1F29),a
    ld a,$C3
    ld ($1F3F),a
done:
    jr done
