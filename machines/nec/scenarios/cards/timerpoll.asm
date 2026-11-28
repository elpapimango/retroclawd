; pce-timerpoll: starts the timer with reload 0 and polls its counter the way Battle Royale does (`lda $0C00 / beq`), counting
; the polls in $30-$32 until a read is not 0, which it stores in $33; then repeats that seven more times ($34.. in groups of 4).
; -D IRQ=1 also enables the timer interrupt with a handler that only acknowledges it.
    .include "common.inc"
    .ifndef IRQ
IRQ = 0
    .endif
    .org $E000
start:
    preamble
    .if IRQ
    lda #$01
    sta $1402
    .endif
    lda #$00
    sta $0C00
    lda #$01
    sta $0C01
    sta $1403
    .if IRQ
    cli
    .endif
    ldx #$00
round:
    stz $30,x
    stz $31,x
    stz $32,x
poll:
    lda $0C00
    bne got
    inc $30,x
    bne poll
    inc $31,x
    bne poll
    inc $32,x
    bne poll
got:
    sta $33,x
    inx
    inx
    inx
    inx
    cpx #$20
    bne round
spin:
    bra spin
isr:
    sta $1403
    rti
    .org $FFF6
    .word $EAEA,$EAEA,isr,$EAEA,start
