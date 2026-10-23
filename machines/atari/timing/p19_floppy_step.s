; p19_floppy_step.s: how long a seek of 20 tracks takes at each of the four step rates (type I command bits 1-0), and a
; restore from track 20 at the 3 ms rate, in loop counts of a loop polling the controller's interrupt request.
; Result at $C000: header 'P19', then five longs: seek rate 0, 1, 2, 3, and the restore.
    .org $90000
    move.w  #$2700,sr
    clr.b   $fffa07             ; IERA, IERB, IMRA, IMRB: no MFP interrupt reaches the CPU
    clr.b   $fffa09
    clr.b   $fffa13
    clr.b   $fffa15
    lea     $c000,a2
    move.l  #$50313900,(a2)+
    moveq   #0,d6               ; the step rate bits
    moveq   #3,d7
rate:
    bsr.w   restore
    move.w  #$086,$ff8606       ; the data register: the track to seek to
    move.w  #20,$ff8604
    move.w  #$080,$ff8606       ; the command register
    moveq   #0,d5
    move.w  d6,d0
    or.w    #$10,d0             ; seek, no verify, no head load
    move.w  d0,$ff8604
rwait:
    addq.l  #1,d5
    btst    #5,$fffa01
    bne.s   rwait
    move.l  d5,(a2)+
    move.w  $ff8604,d0          ; reading the status clears the request
    addq.w  #1,d6
    dbra    d7,rate
    ; a restore from track 20
    moveq   #0,d5
    move.w  #$080,$ff8606
    move.w  #$03,$ff8604
rwait2:
    addq.l  #1,d5
    btst    #5,$fffa01
    bne.s   rwait2
    move.l  d5,(a2)+
    move.w  $ff8604,d0
done:
    bra.s   done

restore:
    move.w  #$080,$ff8606
    move.w  #$03,$ff8604
r1:
    btst    #5,$fffa01
    bne.s   r1
    move.w  $ff8604,d0
    rts
