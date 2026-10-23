; Standard 8K CAR: the native OS runs the entry and calls the init vector.
.cpu 6502
.org $a000
start:
    sei
    lda #0
    sta $d40e
    ldx #0
pattern:
    txa
    eor #$a5
    sta $4000,x
    inx
    bne pattern
done:
    jmp done
init:
    rts
padding:
    .fill $bffa-padding,$ff
    .word start
    .byte 0,4
    .word init
