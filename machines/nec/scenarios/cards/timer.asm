; pce-timer*: starts the timer with RELOAD and stores 48 reads of its counter, about 110 cycles apart, in zero page $20-$4F.
; -D RELOAD=n (default 0), -D IRQ=1 enables the timer interrupt (the handler acknowledges it), -D ACK=1 acknowledges once at start.
    .include "common.inc"
    .ifndef RELOAD
RELOAD = 0
    .endif
    .ifndef IRQ
IRQ = 0
    .endif
    .ifndef ACK
ACK = 0
    .endif
    .org $E000
start:
    preamble
    lda $0C00
    sta $50                ; the counter before the timer is ever started
    .if IRQ
    lda #$01
    sta $1402              ; IRQ2 off, IRQ1 and the timer on
    .endif
    lda #RELOAD
    sta $0C00
    lda #$01
    sta $0C01
    .if ACK
    sta $1403
    .endif
    .if IRQ
    cli
    .endif
    ldx #$00
loop:
    lda $0C00
    sta $20,x
    ldy #$18
delay:
    dey
    bne delay
    inx
    cpx #$30
    bne loop
spin:
    bra spin
    .if IRQ
isr:
    sta $1403              ; acknowledge
    rti
    .endif
    .org $FFF6
    .word $EAEA,$EAEA
    .if IRQ
    .word isr
    .else
    .word $EAEA
    .endif
    .word $EAEA,start
