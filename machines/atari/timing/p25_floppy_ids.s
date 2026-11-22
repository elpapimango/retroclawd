; p25_floppy_ids.s: the order of the sector headers on a track, and the time between them.
; Sixteen Read Address commands in a row, each with the DMA set up for the six header bytes at $020000 (sector count 1: the
; DMA moves 512 bytes, of which the controller delivers six); stored for each: the loop count from the command to the
; controller's interrupt request (a long) and the header's track, side, sector and size bytes (a long, high byte first).
; Result at $C000: header 'P25', then 16 pairs of longs.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50323500,(a2)+
    moveq   #15,d7
each:
    move.l  #$ffffffff,$20000
    move.b  #$02,$ff8609
    move.b  #$00,$ff860b
    move.b  #$00,$ff860d
    move.w  #$190,$ff8606
    move.w  #$090,$ff8606
    move.w  #1,$ff8604
    move.w  #$080,$ff8606
    moveq   #0,d5
    move.w  #$C0,$ff8604
rwait:
    addq.l  #1,d5
    btst    #5,$fffa01
    bne.s   rwait
    move.l  d5,(a2)+
    move.l  $20000,(a2)+
    move.w  $ff8604,d0          ; the status clears the request
    dbra    d7,each
done:
    bra.s   done
