; P46: a Sega Mouse in port 1. After 10 frames and again after 30, the nibbles of one packet are read with the TH/TR handshake (TH low
; starts it, every toggle of TR advances it, TL follows TR when the mouse has answered). The mouse is moved in frame 8 and in frame 28.
; Result at RAM offset 0: 9 nibbles after frame 10, 9 after frame 30, and a count of the handshake waits that timed out.
    .org $200
    lea     $FF0000,a2
    move.b  #$60,$A10009
    move.b  #$60,$A10003
    moveq   #10,d5
    bsr.w   frames
    bsr.w   packet
    moveq   #20,d5
    bsr.w   frames
    bsr.w   packet
    move.b  d3,(a2)+
forever:
    bra.s   forever
frames:
fw: move.b  $C00008,d0
    cmp.b   #100,d0
    bne.s   fw
fx: move.b  $C00008,d0
    cmp.b   #100,d0
    beq.s   fx
    subq.w  #1,d5
    bne.s   fw
    rts
packet:
    move.b  #$60,$A10003
    move.b  #$20,$A10003
    move.b  #$20,d4
    moveq   #8,d6
nib:
    move.w  #3000,d7
w:  move.b  $A10003,d0
    move.b  d4,d1
    lsr.b   #1,d1
    eor.b   d0,d1
    btst    #4,d1
    beq.s   got
    dbf     d7,w
    addq.b  #1,d3
got:
    and.b   #$0F,d0
    move.b  d0,(a2)+
    eor.b   #$20,d4
    move.b  d4,$A10003
    dbf     d6,nib
    move.b  #$60,$A10003
    rts
