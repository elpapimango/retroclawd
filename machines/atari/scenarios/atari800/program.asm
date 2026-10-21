.cpu 6502
.org $2000
; Standalone raw Program. POKEY divisors: Altirra Hardware Reference 5.4.
    sei
    lda #0
    sta $d40e
    sta $d400
    sta $d208
    lda #3
    sta $d20f
    lda #150
    sta $d200
    lda #$af
    sta $d201
    lda #0
    sta $d209
    ldx #0
pattern:
    txa
    eor #$a5
    sta $4000,x
    inx
    bne pattern
done:
    jmp done
