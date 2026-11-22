.cpu 6502
.org $0700
; Standard one-sector Atari boot header, followed by a standalone Program.
.byte 0, 1
.word $0700, $0706
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
.align 128, 0
