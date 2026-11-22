; p20_ikbd_latency.s: how long the keyboard controller takes to answer, byte by byte.
; Twelve commands are sent to the keyboard ACIA ($FFFC00) and the loop counts from the last command byte (then from each
; answer byte) to the next answer byte are stored, with the byte received. A slot that stays empty for 100 000 loops is stored
; as that count and ends the case.
; Result at $C000: header 'P20', then per case 8 longs (loop counts) and 8 bytes (the answers): 12 cases of 40 bytes.
; Cases: reset ($80 $01, answers $F1), joystick interrogate ($16, $FD + 2), time of day ($1C, $FC + 6), mouse position ($0D, $F7 + 4),
; and the status inquiries $87, $88, $89, $8A, $8B, $8F, $90, $92 (an $F6 packet of 8 bytes).
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50323000,(a2)+
    lea     $fffc00,a3
    lea     cases(pc),a4
    moveq   #11,d7
case:
    bsr.w   drain
    moveq   #0,d0
    move.b  (a4)+,d0            ; the number of command bytes
    subq.w  #1,d0
sendloop:
    move.b  (a4)+,d1
    bsr.w   send
    dbra    d0,sendloop
    ; eight answer slots
    lea     (a2),a5             ; the counts
    lea     32(a2),a6           ; the bytes
    moveq   #7,d6
    moveq   #0,d5
    moveq   #0,d4               ; set when a slot timed out
slot:
    tst.b   d4
    bne.s   skip
wait:
    addq.l  #1,d5
    btst    #0,(a3)
    bne.s   got
    cmp.l   #100000,d5
    blt.s   wait
    moveq   #1,d4               ; timed out: this slot holds the cap
    move.l  d5,(a5)+
    clr.b   (a6)+
    moveq   #0,d5
    bra.s   next
got:
    move.l  d5,(a5)+
    move.b  2(a3),(a6)+
    moveq   #0,d5
    bra.s   next
skip:
    clr.l   (a5)+
    clr.b   (a6)+
next:
    dbra    d6,slot
    lea     40(a2),a2
    dbra    d7,case
done:
    bra.s   done

send:                           ; d1 = byte
    btst    #1,(a3)
    beq.s   send
    move.b  d1,2(a3)
    rts

drain:                          ; reads and drops whatever is waiting
    btst    #0,(a3)
    beq.s   drained
    move.b  2(a3),d2
    bra.s   drain
drained:
    rts

cases:
    dc.b    2,$80,$01
    dc.b    1,$16
    dc.b    1,$1C
    dc.b    1,$0D
    dc.b    1,$87
    dc.b    1,$88
    dc.b    1,$89
    dc.b    1,$8A
    dc.b    1,$8B
    dc.b    1,$8F
    dc.b    1,$90
    dc.b    1,$92
