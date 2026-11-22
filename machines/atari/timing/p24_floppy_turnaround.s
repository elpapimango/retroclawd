; p24_floppy_turnaround.s: how soon after a sector read ends the next sector's header can still be caught.
; Eight Read Sector commands in a row (sectors 1 to 8, one sector each, the DMA and the controller set up again before every one)
; with a pause between a command's completion and the set-up of the next one; the pause is d runs of a 12-clock loop for each d of
; a table. The loop counts of the waits for the controller's interrupt request, summed over the eight commands, are stored as a
; long per pause: a missed header costs a revolution (about 30 000 runs).
; Result at $C000: header 'P24', then one long for each of the 12 pauses.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50323400,(a2)+
    lea     pauses(pc),a4
    moveq   #11,d7
each:
    moveq   #0,d4               ; the sum of the waits
    move.w  (a4)+,d3            ; this pause
    moveq   #1,d2               ; the sector
    moveq   #7,d6
sector:
    ; set up the DMA for one sector at $020000
    move.b  #$02,$ff8609
    move.b  #$00,$ff860b
    move.b  #$00,$ff860d
    move.w  #$190,$ff8606
    move.w  #$090,$ff8606
    move.w  #1,$ff8604
    move.w  #$084,$ff8606       ; the sector register
    move.w  d2,$ff8604
    move.w  #$080,$ff8606       ; the command register
    moveq   #0,d5
    move.w  #$80,$ff8604        ; read sector
rwait:
    addq.l  #1,d5
    btst    #5,$fffa01
    bne.s   rwait
    add.l   d5,d4
    move.w  $ff8604,d0          ; the status clears the request
    addq.w  #1,d2
    move.w  d3,d1
pause:
    dbra    d1,pause
    dbra    d6,sector
    move.l  d4,(a2)+
    ; back to sector 1's neighbourhood: a restore keeps the head where it is, so just let the disk turn a little
    move.w  #6000,d1
turn:
    move.w  #$80,d0
    dbra    d1,turn
    dbra    d7,each
done:
    bra.s   done

pauses:
    dc.w    0,10,20,40,60,80,100,130,160,200,250,300
