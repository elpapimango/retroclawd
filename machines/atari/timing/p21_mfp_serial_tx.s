; p21_mfp_serial_tx.s: how long the MFP's transmitter takes for a byte at 9600 baud (timer D, prescale 4, data 2, 8N1,
; divide by 16) and at 1200 baud (data 16).
; For each rate eight bytes are written to the transmit data register one after the other, each as soon as the buffer is
; empty (TSR bit 7); the loop count from a write to the next time the buffer is empty is stored as a long.
; Result at $C000: header 'P21', then 8 longs for 9600 baud, 8 longs for 1200 baud.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50323100,(a2)+
    lea     $fffa00,a3
    move.b  #$88,$29(a3)        ; UCR: divide by 16, 8 bits, 1 stop bit, no parity
    moveq   #2,d3               ; timer D data for 9600 baud
    bsr.w   rate
    moveq   #16,d3              ; and for 1200
    bsr.w   rate
done:
    bra.s   done

rate:
    move.b  d3,$25(a3)          ; TDDR
    move.b  $1d(a3),d0
    and.b   #$70,d0
    or.b    #$01,d0             ; timer D: prescale 4
    move.b  d0,$1d(a3)
    move.b  #$01,$2d(a3)        ; TSR: transmitter enabled
    moveq   #7,d7
    move.b  #$55,$2f(a3)        ; the first byte
txbyte:
    moveq   #0,d5
txwait:
    addq.l  #1,d5
    btst    #7,$2d(a3)
    beq.s   txwait
    move.l  d5,(a2)+
    move.b  #$55,$2f(a3)        ; the next byte
    dbra    d7,txbyte
    ; let the last byte go out and stop the transmitter
    move.w  #4000,d6
drain:
    btst    #7,$2d(a3)
    dbra    d6,drain
    move.b  #$00,$2d(a3)
    rts
