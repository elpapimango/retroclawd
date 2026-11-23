; Shared by the PC Engine test cards: the start-up every card does, and small macros.
    .cpu huc6280

; mapper: MPR0 = hardware page ($FF), MPR1 = work RAM ($F8); stack at the top of the zero-page RAM
.macro preamble
    sei
    csh
    lda #$F8
    tam #$02
    lda #$FF
    tam #$01
    ldx #$FF
    txs
.endm

; store a byte to a hardware register
.macro put value,reg
    lda #value
    sta reg
.endm

; a video controller register: select it, then write the low and high bytes of its data
.macro vdc reg,lo,hi
    st0 #reg
    lda #lo
    sta $0002
    lda #hi
    sta $0003
.endm
